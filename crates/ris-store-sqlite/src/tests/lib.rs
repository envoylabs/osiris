use super::*;
use ris_core::command::{Command, apply, seal};
use ris_core::ids::{AgentName, CheckId, ItemId};
use ris_core::primitives::ItemState;
use ris_core::state::State;
use ris_core::text::{Policy, Text};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: u64 = 1_700_000_000_000;

fn t(s: &str) -> Text {
    Text::new("test", s, &Policy::default()).unwrap()
}

fn salts() -> impl FnMut() -> Salt {
    let mut n: u8 = 0;
    move || {
        n = n.wrapping_add(1);
        Salt([n; 16])
    }
}

fn temp_db(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ris-store-test-{}-{n}", std::process::id()));
    // Process ids are reused: an earlier run may have left a store
    // here, and a test must start from an empty one.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.sqlite"))
}

/// Load, apply, seal, append: what the CLI does per command.
fn run(
    store: &mut SqliteStore,
    c: &Command,
    salt: &mut impl FnMut() -> Salt,
) -> Result<(), StoreError> {
    let state = store.load()?;
    let effects = apply(&state, c).map_err(|r| StoreError::Backend(r.to_string()))?;
    store.append(&seal(&state, effects, NOW, salt))
}

fn scripted(store: &mut SqliteStore) {
    let mut s = salts();
    run(store, &Command::PhaseOpen { goal: t("slice") }, &mut s).unwrap();
    run(
        store,
        &Command::ItemAdd {
            goal: t("first"),
            checks: vec![CheckId("test".into())],
            depends_on: vec![],
        },
        &mut s,
    )
    .unwrap();
    run(
        store,
        &Command::Claim {
            item: ItemId(1),
            agent: AgentName("heron".into()),
        },
        &mut s,
    )
    .unwrap();
}

#[test]
fn appends_persist_and_reload_to_the_same_state() {
    let path = temp_db("persist");
    let live = {
        let mut store = SqliteStore::open(&path).unwrap();
        scripted(&mut store);
        store.load().unwrap()
    };
    let reopened = SqliteStore::open(&path).unwrap();
    let log = reopened.read_all().unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(State::fold(&log).unwrap(), live);
    assert_eq!(reopened.load().unwrap(), live);
    assert_eq!(reopened.head().unwrap(), (3, log[2].hash));
}

#[test]
fn an_append_that_does_not_follow_the_head_is_a_conflict() {
    let mut store = SqliteStore::in_memory().unwrap();
    scripted(&mut store);
    let stale = {
        let log = store.read_all().unwrap();
        State::fold(&log[..2]).unwrap()
    };
    let effects = apply(
        &stale,
        &Command::Claim {
            item: ItemId(1),
            agent: AgentName("wren".into()),
        },
    )
    .unwrap();
    let events = seal(&stale, effects, NOW, salts());
    let err = store.append(&events).unwrap_err();
    assert!(matches!(err, StoreError::Conflict { len: 3, .. }));
    assert_eq!(store.head().unwrap().0, 3, "nothing was appended");
}

#[test]
fn concurrent_claimers_from_separate_connections_get_one_winner() {
    let path = temp_db("concurrent");
    {
        let mut store = SqliteStore::open(&path).unwrap();
        let mut s = salts();
        run(&mut store, &Command::PhaseOpen { goal: t("slice") }, &mut s).unwrap();
        run(
            &mut store,
            &Command::ItemAdd {
                goal: t("contested"),
                checks: vec![],
                depends_on: vec![],
            },
            &mut s,
        )
        .unwrap();
    }
    let agents = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let handles: Vec<_> = agents
        .iter()
        .map(|name| {
            let path = path.clone();
            let name = (*name).to_owned();
            std::thread::spawn(move || {
                let mut store = SqliteStore::open(&path).unwrap();
                let mut salt = {
                    let seed = name.as_bytes()[0];
                    let mut n = seed;
                    move || {
                        n = n.wrapping_add(1);
                        Salt([n; 16])
                    }
                };
                // Retry on conflict, as the CLI will; stop on a refusal.
                for _ in 0..50 {
                    let state = store.load().unwrap();
                    let c = Command::Claim {
                        item: ItemId(1),
                        agent: AgentName(name.clone()),
                    };
                    let effects = match apply(&state, &c) {
                        Ok(e) => e,
                        Err(_) => return false,
                    };
                    match store.append(&seal(&state, effects, NOW, &mut salt)) {
                        Ok(()) => return true,
                        Err(StoreError::Conflict { .. }) => continue,
                        Err(e) => panic!("{e}"),
                    }
                }
                panic!("never settled");
            })
        })
        .collect();
    let wins = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|&w| w)
        .count();
    assert_eq!(wins, 1, "exactly one claim succeeds");
    let store = SqliteStore::open(&path).unwrap();
    let state = store.load().unwrap();
    assert_eq!(state.len, 3, "one claim event, no gaps, no extras");
    assert!(matches!(
        state.items[&ItemId(1)].state,
        ItemState::Claimed { .. }
    ));
    ris_core::event::verify(&store.read_all().unwrap()).unwrap();
}

#[test]
fn a_file_with_another_schema_version_is_refused() {
    let path = temp_db("schema");
    {
        let store = SqliteStore::open(&path).unwrap();
        store
            .conn
            .execute(
                "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
    }
    let err = match SqliteStore::open(&path) {
        Ok(_) => panic!("a foreign schema version must be refused"),
        Err(e) => e,
    };
    assert!(matches!(err, StoreError::Backend(ref m) if m.contains("schema version 99")));
}

/// A schema-1 file, written as the first release wrote it: no `at`
/// column, format-version-1 events.
#[test]
fn a_schema_1_file_is_upgraded_in_place() {
    let path = temp_db("schema1");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO meta VALUES ('schema_version', '1');
                 CREATE TABLE events (
                     seq INTEGER PRIMARY KEY, version INTEGER NOT NULL, prev BLOB NOT NULL,
                     structure TEXT NOT NULL, text_hash BLOB NOT NULL, salt BLOB, text TEXT,
                     hash BLOB NOT NULL);",
        )
        .unwrap();
        // Build two v1 events by hand with v1's hash rule.
        use sha2::{Digest, Sha256};
        fn digest(domain: &str, parts: &[&[u8]]) -> [u8; 32] {
            let mut h = Sha256::new();
            h.update(domain.as_bytes());
            for p in parts {
                h.update((p.len() as u64).to_be_bytes());
                h.update(p);
            }
            h.finalize().into()
        }
        let mut prev = [0u8; 32];
        // The structure as format version 1 stored it: the old JSON
        // form, written out, since the old types no longer exist.
        let steps = [
            (
                r#"{"event":"PhaseOpened","phase":1}"#,
                r#"{"goal":"old phase"}"#,
            ),
            (
                r#"{"event":"ItemAdded","item":1,"phase":1,"checks":[],"depends_on":[]}"#,
                r#"{"goal":"old item"}"#,
            ),
        ];
        for (i, (structure_json, text)) in steps.iter().enumerate() {
            let salt = [i as u8 + 1; 16];
            let text_hash = digest("ris-text-v1", &[&salt, text.as_bytes()]);
            let hash = digest(
                "ris-event-v1",
                &[&prev, structure_json.as_bytes(), &text_hash],
            );
            conn.execute(
                "INSERT INTO events VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    i as i64 + 1,
                    prev.as_slice(),
                    structure_json,
                    text_hash.as_slice(),
                    salt.as_slice(),
                    text,
                    hash.as_slice()
                ],
            )
            .unwrap();
            prev = hash;
        }
    }
    let store = SqliteStore::open(&path).unwrap();
    let log = store.read_all().unwrap();
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|e| e.version == 3 && e.at == 0));
    ris_core::event::verify(&log).unwrap();
    let state = store.load().unwrap();
    assert_eq!(state.items.len(), 1);
    let version: String = store
        .conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, "3");
    // Reopening is a no-op, and new events carry their time.
    drop(store);
    let mut store = SqliteStore::open(&path).unwrap();
    run(&mut store, &Command::PhaseClose, &mut salts()).unwrap();
    let log = store.read_all().unwrap();
    assert_eq!(log[2].at, NOW);
}

#[test]
fn redacted_text_round_trips_as_null() {
    let mut store = SqliteStore::in_memory().unwrap();
    let state = State::default();
    let effects = apply(
        &state,
        &Command::PhaseOpen {
            goal: t("secretish"),
        },
    )
    .unwrap();
    let events: Vec<Event> = seal(&state, effects, NOW, salts())
        .into_iter()
        .map(Event::redacted)
        .collect();
    store.append(&events).unwrap();
    let back = store.read_all().unwrap();
    assert_eq!(back, events);
    assert_eq!(
        ris_core::event::verify(&back).unwrap().missing_text,
        vec![1]
    );
}

#[test]
fn the_merge_lock_excludes_and_is_released_on_drop() {
    let path = temp_db("lock");
    let held = MergeLock::acquire(&path, std::time::Duration::from_millis(0)).unwrap();
    assert!(
        MergeLock::acquire(&path, std::time::Duration::from_millis(50)).is_err(),
        "a second holder waits, then gives up"
    );
    drop(held);
    MergeLock::acquire(&path, std::time::Duration::from_millis(0)).unwrap();
}

/// The append-path bench: at least a hundred times five agents'
/// peak, taken as ten events a second, so 1,000 events a second on
/// a file-backed store with one transaction per event.
#[test]
fn append_path_has_headroom() {
    let path = temp_db("bench");
    let mut store = SqliteStore::open(&path).unwrap();
    let mut salt = salts();
    let mut state = State::default();
    let n = 2_000u64;
    let started = std::time::Instant::now();
    for i in 0..n {
        let c = if i == 0 {
            Command::PhaseOpen { goal: t("bench") }
        } else {
            Command::ItemAdd {
                goal: t("an item with a goal of ordinary length, say this long"),
                checks: vec![CheckId("test".into())],
                depends_on: vec![],
            }
        };
        let events = seal(&state, apply(&state, &c).unwrap(), NOW, &mut salt);
        store.append(&events).unwrap();
        for e in &events {
            state.apply_event(e).unwrap();
        }
    }
    let secs = started.elapsed().as_secs_f64();
    let rate = n as f64 / secs;
    eprintln!("append path: {rate:.0} events/s ({n} events in {secs:.2}s)");
    assert!(rate >= 1_000.0, "append path too slow: {rate:.0} events/s");
    // The CLI loads (reads, verifies, folds) the whole log before
    // each command; report what that costs at this size.
    let started = std::time::Instant::now();
    let loaded = store.load().unwrap();
    assert_eq!(loaded, state);
    eprintln!(
        "load of {n} events: {:.1}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}
