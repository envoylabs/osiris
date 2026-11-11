//! SQLite adapter for the `Store` port of `ris-core`.
//!
//! One file, WAL mode, one table of events. Each append is an immediate
//! transaction that re-reads the head and refuses if the events do not
//! follow it, so any number of processes may hold connections and
//! exactly one of two concurrent appends after the same head succeeds.
//! Event parts are stored as JSON so the `sqlite3` tool can read them.
//!
//! Schema history: version 1 stored format-version-1 events (no `at`);
//! version 2 stored format-version-2 events; version 3 stores format
//! version 3, whose JSON is the shown form. Opening an older file
//! upgrades it in place, inside one transaction: the old chain is
//! verified from the stored bytes, the log is rechained under the
//! current format, and the schema version is set to 3.

use std::path::Path;

use ris_core::event::{Event, Salt, Structure, TextPart};
use ris_core::ids::Hash;
use ris_core::store::{Store, StoreError, check_follows};
use rusqlite::{Connection, OptionalExtension, params};

/// The schema this build writes. A file with another version is refused.
pub const SCHEMA_VERSION: i64 = 3;

/// A store backed by one SQLite file.
pub struct SqliteStore {
    conn: Connection,
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

impl SqliteStore {
    /// Open or create the store at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        Self::init(conn)
    }

    /// An in-memory store, for tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory().map_err(backend)?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(backend)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(backend)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(backend)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                 seq       INTEGER PRIMARY KEY,
                 version   INTEGER NOT NULL,
                 prev      BLOB    NOT NULL,
                 at        INTEGER NOT NULL DEFAULT 0,
                 structure TEXT    NOT NULL,
                 text_hash BLOB    NOT NULL,
                 salt      BLOB,
                 text      TEXT,
                 hash      BLOB    NOT NULL
             );",
        )
        .map_err(backend)?;
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        match version {
            None => {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )
                .map_err(backend)?;
            }
            Some(v) if v == SCHEMA_VERSION.to_string() => {}
            Some(v) if v == "1" || v == "2" => Self::upgrade_old(&conn)?,
            Some(v) => {
                return Err(StoreError::Backend(format!(
                    "store has schema version {v}; this build reads {SCHEMA_VERSION}"
                )));
            }
        }
        Ok(Self { conn })
    }

    /// Schemas 1 and 2 held older event formats (schema 1 without
    /// `at`). Add the column if missing, verify and rechain the log
    /// under the current format, and bump the schema, in one
    /// transaction.
    fn upgrade_old(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(backend)?;
        let result = (|| {
            let has_at: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'at'",
                    [],
                    |r| r.get(0),
                )
                .map_err(backend)?;
            if has_at == 0 {
                conn.execute_batch("ALTER TABLE events ADD COLUMN at INTEGER NOT NULL DEFAULT 0")
                    .map_err(backend)?;
            }
            let old = Self::read_raw_on(conn)?;
            let new = ris_core::event::upgrade(&old)?;
            conn.execute_batch("DELETE FROM events").map_err(backend)?;
            Self::insert_on(conn, &new)?;
            conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(backend)?;
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(backend),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn insert_on(conn: &Connection, events: &[Event]) -> Result<(), StoreError> {
        let mut insert = conn
            .prepare_cached(
                "INSERT INTO events
                 (seq, version, prev, at, structure, text_hash, salt, text, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(backend)?;
        for e in events {
            let structure = serde_json::to_string(&e.structure).map_err(backend)?;
            let text = match &e.text {
                Some(t) => Some(serde_json::to_string(t).map_err(backend)?),
                None => None,
            };
            insert
                .execute(params![
                    e.seq as i64,
                    i64::from(e.version),
                    e.prev.0.as_slice(),
                    e.at as i64,
                    structure,
                    e.text_hash.0.as_slice(),
                    e.salt.as_ref().map(|s| s.0.as_slice()),
                    text,
                    e.hash.0.as_slice(),
                ])
                .map_err(backend)?;
        }
        Ok(())
    }

    fn read_from_on(conn: &Connection, from: u64) -> Result<Vec<Event>, StoreError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT seq, version, prev, at, structure, text_hash, salt, text, hash
                 FROM events WHERE seq >= ?1 ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![from as i64], row_to_event)
            .map_err(backend)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(backend)??);
        }
        Ok(events)
    }

    /// The log as stored, the structure kept as its stored text, for an
    /// upgrade to verify against the old chain.
    fn read_raw_on(conn: &Connection) -> Result<Vec<ris_core::RawEvent>, StoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT seq, version, prev, at, structure, text_hash, salt, text, hash
                 FROM events ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, Option<Vec<u8>>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Vec<u8>>(8)?,
                ))
            })
            .map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, version, prev, at, structure, text_hash, salt, text, hash) =
                row.map_err(backend)?;
            let salt = match salt {
                Some(s) => Some(Salt(s.as_slice().try_into().map_err(|_| {
                    StoreError::Backend(format!("event {seq}: salt is not 16 bytes"))
                })?)),
                None => None,
            };
            let text: Option<TextPart> = match text {
                Some(t) => Some(serde_json::from_str(&t).map_err(backend)?),
                None => None,
            };
            out.push(ris_core::RawEvent {
                seq: seq as u64,
                version: version as u16,
                prev: hash_from(&prev)?,
                at: at as u64,
                structure,
                text_hash: hash_from(&text_hash)?,
                salt,
                text,
                hash: hash_from(&hash)?,
            });
        }
        Ok(out)
    }

    fn head_on(conn: &Connection) -> Result<(u64, Hash), StoreError> {
        let row: Option<(i64, Vec<u8>)> = conn
            .query_row(
                "SELECT seq, hash FROM events ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(backend)?;
        match row {
            None => Ok((0, Hash::ZERO)),
            Some((seq, hash)) => Ok((seq as u64, hash_from(&hash)?)),
        }
    }
}

fn hash_from(bytes: &[u8]) -> Result<Hash, StoreError> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StoreError::Backend("a stored hash is not 32 bytes".into()))?;
    Ok(Hash(arr))
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Event, StoreError>> {
    let seq: i64 = r.get(0)?;
    let version: i64 = r.get(1)?;
    let prev: Vec<u8> = r.get(2)?;
    let at: i64 = r.get(3)?;
    let structure: String = r.get(4)?;
    let text_hash: Vec<u8> = r.get(5)?;
    let salt: Option<Vec<u8>> = r.get(6)?;
    let text: Option<String> = r.get(7)?;
    let hash: Vec<u8> = r.get(8)?;
    Ok((|| {
        let structure: Structure = serde_json::from_str(&structure).map_err(backend)?;
        let text: Option<TextPart> = match text {
            Some(t) => Some(serde_json::from_str(&t).map_err(backend)?),
            None => None,
        };
        let salt = match salt {
            Some(s) => Some(Salt(s.try_into().map_err(|_| {
                StoreError::Backend("a stored salt is not 16 bytes".into())
            })?)),
            None => None,
        };
        Ok(Event {
            seq: seq as u64,
            version: version as u16,
            prev: hash_from(&prev)?,
            at: at as u64,
            structure,
            text_hash: hash_from(&text_hash)?,
            salt,
            text,
            hash: hash_from(&hash)?,
        })
    })())
}

impl Store for SqliteStore {
    fn append(&mut self, events: &[Event]) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let (len, head) = Self::head_on(&tx)?;
        check_follows(len, head, events)?;
        Self::insert_on(&tx, events)?;
        tx.commit().map_err(backend)
    }

    fn read_from(&self, from: u64) -> Result<Vec<Event>, StoreError> {
        Self::read_from_on(&self.conn, from)
    }

    fn head(&self) -> Result<(u64, Hash), StoreError> {
        Self::head_on(&self.conn)
    }
}

/// A lock held for the length of one merge, so merges happen one at a
/// time. It is an exclusive transaction on its own SQLite file: other
/// processes wait up to `wait` for it, and the operating system releases
/// it if the holding process dies, so a crash never leaves it stuck.
pub struct MergeLock {
    _conn: Connection,
}

impl MergeLock {
    pub fn acquire(path: &Path, wait: std::time::Duration) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        conn.busy_timeout(wait).map_err(backend)?;
        conn.execute_batch("BEGIN EXCLUSIVE")
            .map_err(|e| StoreError::Backend(format!("another merge holds the lock: {e}")))?;
        Ok(Self { _conn: conn })
    }
}

#[cfg(test)]
mod tests {
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
}
