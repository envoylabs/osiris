use super::*;
use crate::text::Policy;

fn text(pairs: &[(&str, &str)]) -> TextPart {
    pairs
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_owned(),
                Text::new(k, *v, &Policy::default()).unwrap(),
            )
        })
        .collect()
}

fn salt(n: u8) -> Salt {
    Salt([n; 16])
}

fn sample_log() -> Vec<Event> {
    let mut log = Vec::new();
    let mut prev = Hash::ZERO;
    let steps: Vec<(Structure, TextPart)> = vec![
        (
            Structure::PhaseOpened { phase: PhaseId(1) },
            text(&[("goal", "first slice")]),
        ),
        (
            Structure::ItemAdded {
                item: ItemId(1),
                phase: PhaseId(1),
                checks: vec![CheckId("cargo-test".into())],
                depends_on: vec![],
            },
            text(&[("goal", "the event envelope")]),
        ),
        (
            Structure::ItemClaimed {
                item: ItemId(1),
                agent: AgentName("heron".into()),
            },
            TextPart::new(),
        ),
        (
            Structure::DecisionRaised {
                decision: DecisionId(1),
                class: DecisionClass::Mechanical,
                by: Actor::Agent(AgentName("heron".into())),
                about: Some(ItemId(1)),
                options: 2,
                recommended: 0,
            },
            text(&[
                ("question", "sha2 or blake3?"),
                ("option.0", "sha2"),
                ("option.1", "blake3"),
            ]),
        ),
    ];
    for (i, (structure, t)) in steps.into_iter().enumerate() {
        let e = Event::new(
            i as u64 + 1,
            prev,
            1_700_000_000_000 + i as u64,
            structure,
            t,
            salt(i as u8),
        );
        prev = e.hash;
        log.push(e);
    }
    log
}

#[test]
fn a_fresh_log_verifies() {
    let log = sample_log();
    let v = verify(&log).unwrap();
    assert_eq!(v.len, 4);
    assert_eq!(v.head, log[3].hash);
    assert!(v.missing_text.is_empty());
    assert_eq!(verify(&[]).unwrap().head, Hash::ZERO);
}

#[test]
fn a_continuation_verifies_against_the_head_it_follows() {
    let log = sample_log();
    let first = verify(&log[..2]).unwrap();
    let rest = verify_after(first.len, first.head, &log[2..]).unwrap();
    assert_eq!((rest.len, rest.head), (4, log[3].hash));
    // Wrong head: the first new event does not link.
    assert_eq!(
        verify_after(first.len, log[0].hash, &log[2..]),
        Err(ChainError::Link { seq: 3 })
    );
    // Wrong length: the sequence does not continue.
    assert!(matches!(
        verify_after(1, first.head, &log[2..]),
        Err(ChainError::Sequence { .. })
    ));
}

#[test]
fn altering_a_structural_field_breaks_the_chain() {
    let mut log = sample_log();
    log[2].structure = Structure::ItemClaimed {
        item: ItemId(1),
        agent: AgentName("wren".into()),
    };
    assert_eq!(verify(&log), Err(ChainError::Hash { seq: 3 }));
}

#[test]
fn altering_text_breaks_the_chain() {
    let mut log = sample_log();
    log[1].text = Some(text(&[("goal", "something else")]));
    assert_eq!(verify(&log), Err(ChainError::TextHash { seq: 2 }));
}

#[test]
fn rewriting_history_breaks_every_later_link() {
    let mut log = sample_log();
    let replaced = Event::new(
        1,
        Hash::ZERO,
        1_700_000_000_000,
        Structure::PhaseOpened { phase: PhaseId(2) },
        text(&[("goal", "rewritten")]),
        salt(9),
    );
    log[0] = replaced;
    assert_eq!(verify(&log), Err(ChainError::Link { seq: 2 }));
}

#[test]
fn a_gap_in_sequence_is_refused() {
    let mut log = sample_log();
    log.remove(1);
    assert_eq!(
        verify(&log),
        Err(ChainError::Sequence {
            at: 1,
            expected: 2,
            found: 3
        })
    );
}

#[test]
fn removed_text_leaves_the_chain_verifying_and_is_reported() {
    let mut log = sample_log();
    log[3] = log[3].clone().redacted();
    let v = verify(&log).unwrap();
    assert_eq!(v.missing_text, vec![4]);
    assert_eq!(v.head, log[3].hash);
    assert!(log[3].text_field("question").is_none());
    assert!(log[1].text_field("goal").is_some());
}

#[test]
fn half_redaction_is_refused() {
    let mut log = sample_log();
    log[3].salt = None;
    assert_eq!(verify(&log), Err(ChainError::HalfRedacted { seq: 4 }));
}

#[test]
fn a_short_text_is_not_recoverable_from_its_hash_by_guessing() {
    // The text is one of a handful of obvious answers. Without the
    // salt, hashing each candidate under the obvious salts does not
    // find it.
    // Named `hidden`, not `secret`: a variable called `secret = ...`
    // reads as a credential assignment to the exit scan.
    let hidden = text(&[("rationale", "yes")]);
    let real_salt = Salt(*b"0123456789abcdef");
    let e = Event::new(
        1,
        Hash::ZERO,
        1_700_000_000_000,
        Structure::DecisionTaken {
            decision: DecisionId(1),
            by: Actor::Person,
            chosen: 0,
        },
        hidden,
        real_salt,
    )
    .redacted();
    let guessed_salts = [Salt([0; 16]), Salt([0xff; 16]), Salt(*b"saltsaltsaltsalt")];
    for candidate in ["yes", "no", "ship it", "later"] {
        let t = text(&[("rationale", candidate)]);
        for s in &guessed_salts {
            assert_ne!(hash_text(s, &t), e.text_hash);
        }
    }
    // With the real salt, the hash is of course reproducible.
    assert_eq!(
        hash_text(&real_salt, &text(&[("rationale", "yes")])),
        e.text_hash
    );
}

#[test]
fn events_round_trip_through_json() {
    let log = sample_log();
    let json = serde_json::to_string(&log).unwrap();
    let back: Vec<Event> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, log);
    assert!(verify(&back).is_ok());
}

#[test]
fn the_hash_is_stable_across_builds() {
    // Pins the canonical form. If this changes, old logs no longer
    // verify, so it is a format version bump, not a refactor.
    let e = Event::new(
        1,
        Hash::ZERO,
        1_700_000_000_000,
        Structure::PhaseOpened { phase: PhaseId(1) },
        text(&[("goal", "first slice")]),
        Salt([7; 16]),
    );
    assert_eq!(
        e.hash.to_hex(),
        "f88155e61ba2108a74b72c2da1d879c4d941ddb2e71dfd4e92a4c4aad8493f5b"
    );
}

#[test]
fn the_stored_form_is_the_shown_form() {
    let s = |st: Structure| serde_json::to_string(&st).unwrap();
    assert_eq!(
        s(Structure::DecisionRaised {
            decision: DecisionId(5),
            class: DecisionClass::Substantive,
            by: Actor::Agent(AgentName("heron".into())),
            about: Some(ItemId(32)),
            options: 2,
            recommended: 0,
        }),
        r#"{"event":"decision_raised","decision":"D-5","class":"substantive","by":{"kind":"agent","seat":"heron"},"about":"I-32","options":2,"recommended":0}"#
    );
    assert_eq!(
        s(Structure::ItemBlocked {
            item: ItemId(1),
            on: vec![crate::primitives::Blocker::Decision(DecisionId(2))],
        }),
        r#"{"event":"item_blocked","item":"I-1","on":[{"kind":"decision","id":"D-2"}]}"#
    );
    let mut h = [0u8; 32];
    h[0] = 0xab;
    let hex = Hash(h).to_hex();
    assert_eq!(
        s(Structure::CheckRan {
            item: ItemId(1),
            check: CheckId("test".into()),
            passed: true,
            evidence: EvidenceId(Hash(h)),
        }),
        format!(
            r#"{{"event":"check_ran","item":"I-1","check":"test","passed":true,"evidence":"E-{hex}"}}"#
        )
    );
    let e = Event::new(
        1,
        Hash::ZERO,
        0,
        Structure::PhaseOpened { phase: PhaseId(1) },
        text(&[("goal", "g")]),
        salt(1),
    );
    let v: serde_json::Value = serde_json::to_value(&e).unwrap();
    assert_eq!(v["prev"], Hash::ZERO.to_hex());
    assert_eq!(v["salt"], "01".repeat(16));
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back, e, "the stored form reads back as the same event");
}

/// The old JSON form of each converted field, with old hashes,
/// built by hand: the old types no longer exist.
fn old_log() -> Vec<RawEvent> {
    let arr = |n: u8| format!("[{}]", vec![n.to_string(); 32].join(","));
    let structures = [
            r#"{"event":"PhaseOpened","phase":1}"#.to_owned(),
            r#"{"event":"ItemAdded","item":1,"phase":1,"checks":["test"],"depends_on":[]}"#.to_owned(),
            r#"{"event":"ItemAdded","item":2,"phase":1,"checks":[],"depends_on":[1]}"#.to_owned(),
            r#"{"event":"ItemClaimed","item":1,"agent":"heron"}"#.to_owned(),
            r#"{"event":"DecisionRaised","decision":1,"class":"Substantive","by":{"Agent":"heron"},"about":1,"options":2,"recommended":0}"#.to_owned(),
            r#"{"event":"ItemBlocked","item":1,"on":[{"Decision":1}]}"#.to_owned(),
            r#"{"event":"DecisionTaken","decision":1,"by":"Person","chosen":0}"#.to_owned(),
            r#"{"event":"ItemReady","item":1}"#.to_owned(),
            format!(r#"{{"event":"CheckRan","item":1,"check":"test","passed":true,"evidence":{}}}"#, arr(7)),
            format!(r#"{{"event":"ItemDone","item":1,"evidence":[{}]}}"#, arr(7)),
            r#"{"event":"ItemMerged","item":1,"commit":"abc"}"#.to_owned(),
            r#"{"event":"ItemAudited","item":1,"by":"wren"}"#.to_owned(),
            r#"{"event":"MessageSent","message":1,"from":{"Agent":"wren"},"to":"heron","kind":"InterfaceChanged","about":null}"#.to_owned(),
            "CLOSE".to_owned(),
        ];
    let mut log = Vec::new();
    let (mut prev, mut report) = (Hash::ZERO, Hash::ZERO);
    for (i, st) in structures.iter().enumerate() {
        let seq = i as u64 + 1;
        let structure = if st == "CLOSE" {
            report = crate::command::report_hash(&report, &prev);
            format!(
                r#"{{"event":"PhaseClosed","phase":1,"report":[{}]}}"#,
                report
                    .0
                    .iter()
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        } else {
            st.clone()
        };
        let t = text(&[("goal", "old text")]);
        let text_hash = hash_text(&salt(i as u8), &t);
        let at = 1_700_000_000_000 + seq;
        let hash = digest(
            "ris-event-v2",
            &[
                &prev.0,
                &at.to_be_bytes(),
                structure.as_bytes(),
                &text_hash.0,
            ],
        );
        log.push(RawEvent {
            seq,
            version: 2,
            prev,
            at,
            structure,
            text_hash,
            salt: Some(salt(i as u8)),
            text: Some(t),
            hash,
        });
        prev = hash;
    }
    log
}

#[test]
fn an_old_log_upgrades_field_by_field_and_its_reports_are_recomputed() {
    let old = old_log();
    let new = upgrade(&old).unwrap();
    let v = verify(&new).unwrap();
    assert_eq!(v.len, old.len() as u64);
    assert!(
        new.iter()
            .zip(&old)
            .all(|(n, o)| n.at == o.at && n.version == FORMAT_VERSION)
    );
    assert_ne!(new.last().unwrap().hash, old.last().unwrap().hash);
    // The upgraded log folds, and its meaning is unchanged.
    let state = crate::state::State::fold(&new).unwrap();
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[&ItemId(2)].depends_on, vec![ItemId(1)]);
    assert_eq!(
        state.items[&ItemId(1)].audited_by,
        vec![AgentName("wren".into())]
    );
    assert_eq!(state.messages[0].kind, MessageKind::InterfaceChanged);
    assert_eq!(
        state.decisions[&DecisionId(1)].by,
        Actor::Agent(AgentName("heron".into()))
    );
    // The report is the one the new chain gives.
    let close = new.last().unwrap();
    let Structure::PhaseClosed { report, .. } = close.structure else {
        panic!()
    };
    assert_eq!(
        report,
        crate::command::report_hash(&Hash::ZERO, &close.prev)
    );
    assert_eq!(state.last_report, report);
}

#[test]
fn a_broken_old_log_is_refused_not_laundered() {
    let mut tampered = old_log();
    tampered[3].structure = r#"{"event":"ItemClaimed","item":1,"agent":"wren"}"#.into();
    assert_eq!(upgrade(&tampered), Err(ChainError::Hash { seq: 4 }));

    // A report that does not match the old chain, rehashed so the
    // chain itself holds: the upgrade still refuses it.
    let mut log = old_log();
    let last = log.len() - 1;
    let e = &mut log[last];
    e.structure = format!(
        r#"{{"event":"PhaseClosed","phase":1,"report":[{}]}}"#,
        vec!["9"; 32].join(",")
    );
    e.hash = digest(
        "ris-event-v2",
        &[
            &e.prev.0,
            &e.at.to_be_bytes(),
            e.structure.as_bytes(),
            &e.text_hash.0,
        ],
    );
    assert_eq!(
        upgrade(&log),
        Err(ChainError::Report {
            seq: last as u64 + 1
        })
    );

    let mut v1 = old_log();
    v1[0].version = 7;
    assert_eq!(
        upgrade(&v1),
        Err(ChainError::Version { seq: 1, version: 7 })
    );
}
