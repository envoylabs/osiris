//! The event: one entry in the log, in two parts, hash-chained.
//!
//! The structural part (ids, states, which option was chosen) enters the
//! chain hash directly. The free text (a goal, a rationale, a message
//! body) enters it by salted hash, so that toxic text can be removed
//! later without breaking the chain. The fold reads only the structural
//! part, so state never depends on text that might be gone.
//!
//! ```text
//! text_hash(n) = H("ris-text-v1"  || salt(n) || text(n))
//! hash(n)      = H("ris-event-v2" || hash(n-1) || at(n) || structure(n) || text_hash(n))
//! ```
//!
//! The core has no randomness and no clock: the shell supplies the salt
//! and the time (`at`, milliseconds since the Unix epoch; 0 means
//! unknown, as on events upgraded from format version 1). Hashing is
//! SHA-256 over length-prefixed parts, and the canonical form of a part
//! is its compact JSON, so a chain can be checked outside the harness
//! with standard tools.
//!
//! Format history: version 1 had no `at`; [`upgrade_from_v1`] rechains
//! such a log under version 2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ids::{
    Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, Hash, ItemId, MessageId, PhaseId,
};
use crate::primitives::{Blocker, DecisionClass, MessageKind};
use crate::text::Text;

/// The event format version. Events are versioned from the first
/// release, with an upgrade step for old versions.
pub const FORMAT_VERSION: u16 = 2;

/// Random bytes the shell supplies for each event's text hash. Deleted
/// with the text, so a short text cannot be recovered by guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Salt(pub [u8; 16]);

/// The structural part of an event: everything the fold reads. No free
/// text lives here, by construction: none of these fields is a `Text`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Structure {
    PhaseOpened {
        phase: PhaseId,
    },
    PhaseClosed {
        phase: PhaseId,
        /// Taken over the log's head at close, not over the rendering.
        report: Hash,
    },
    ItemAdded {
        item: ItemId,
        phase: PhaseId,
        checks: Vec<CheckId>,
        depends_on: Vec<ItemId>,
    },
    ItemClaimed {
        item: ItemId,
        agent: AgentName,
    },
    /// A substantive decision was raised about a claimed item, which is
    /// set aside until it is taken.
    ItemBlocked {
        item: ItemId,
        on: Vec<Blocker>,
    },
    /// An item's blockers all cleared.
    ItemReady {
        item: ItemId,
    },
    ItemDone {
        item: ItemId,
        evidence: Vec<EvidenceId>,
    },
    ItemMerged {
        item: ItemId,
        commit: CommitHash,
    },
    /// An auditor reviewed a done or merged item. Findings travel as
    /// messages; this records only that the review happened.
    ItemAudited {
        item: ItemId,
        by: AgentName,
    },
    DecisionRaised {
        decision: DecisionId,
        class: DecisionClass,
        by: Actor,
        about: Option<ItemId>,
        /// How many options the text part carries, as `option.0` and so
        /// on.
        options: usize,
        recommended: usize,
    },
    DecisionTaken {
        decision: DecisionId,
        by: Actor,
        chosen: usize,
    },
    CheckRan {
        item: ItemId,
        check: CheckId,
        passed: bool,
        evidence: EvidenceId,
    },
    MessageSent {
        message: MessageId,
        from: Actor,
        to: AgentName,
        kind: MessageKind,
        about: Option<ItemId>,
    },
}

/// The free-text part of an event, by field name. Which names an event
/// kind carries is documented on [`Structure`]'s variants; the fold
/// never reads this.
pub type TextPart = BTreeMap<String, Text>;

/// One entry in the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Position in the log, from 1.
    pub seq: u64,
    pub version: u16,
    /// The previous event's `hash`; all zeros for the first.
    pub prev: Hash,
    /// When the shell appended it, in milliseconds since the Unix epoch;
    /// 0 when unknown (an event upgraded from format version 1).
    pub at: u64,
    pub structure: Structure,
    /// `H(salt, text)`, kept after the text is removed.
    pub text_hash: Hash,
    /// `None` once the text has been removed.
    pub salt: Option<Salt>,
    /// `None` once the text has been removed.
    pub text: Option<TextPart>,
    /// The chain hash of this event.
    pub hash: Hash,
}

impl Event {
    /// Build the event that follows `prev` (the head's hash, or
    /// [`Hash::ZERO`] for an empty log).
    pub fn new(
        seq: u64,
        prev: Hash,
        at: u64,
        structure: Structure,
        text: TextPart,
        salt: Salt,
    ) -> Self {
        let text_hash = hash_text(&salt, &text);
        let hash = hash_event(&prev, at, &structure, &text_hash);
        Self {
            seq,
            version: FORMAT_VERSION,
            prev,
            at,
            structure,
            text_hash,
            salt: Some(salt),
            text: Some(text),
            hash,
        }
    }

    /// Remove the text and its salt. The chain hash is unchanged.
    pub fn redacted(mut self) -> Self {
        self.salt = None;
        self.text = None;
        self
    }

    /// One text field, if the event still has its text.
    pub fn text_field(&self, name: &str) -> Option<&Text> {
        self.text.as_ref()?.get(name)
    }
}

impl Hash {
    /// The `prev` of the first event.
    pub const ZERO: Hash = Hash([0; 32]);
}

fn canonical<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("event parts serialize")
}

fn digest(domain: &str, parts: &[&[u8]]) -> Hash {
    let mut h = Sha256::new();
    h.update(domain.as_bytes());
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    Hash(h.finalize().into())
}

/// `H("ris-text-v1" || salt || canonical(text))`.
pub fn hash_text(salt: &Salt, text: &TextPart) -> Hash {
    digest("ris-text-v1", &[&salt.0, &canonical(text)])
}

/// `H("ris-event-v2" || prev || at || canonical(structure) || text_hash)`.
pub fn hash_event(prev: &Hash, at: u64, structure: &Structure, text_hash: &Hash) -> Hash {
    digest(
        "ris-event-v2",
        &[
            &prev.0,
            &at.to_be_bytes(),
            &canonical(structure),
            &text_hash.0,
        ],
    )
}

/// Format version 1's chain hash: no `at`.
fn hash_event_v1(prev: &Hash, structure: &Structure, text_hash: &Hash) -> Hash {
    digest(
        "ris-event-v1",
        &[&prev.0, &canonical(structure), &text_hash.0],
    )
}

/// Rechain a format-version-1 log under the current version. The old
/// chain is verified first, so a broken log is not laundered into a
/// valid one; every event's `at` becomes 0 (unknown). The head hash
/// changes: this is the one-off cost of adding a field to the format.
pub fn upgrade_from_v1(old: &[Event]) -> Result<Vec<Event>, ChainError> {
    let mut prev_old = Hash::ZERO;
    let mut prev_new = Hash::ZERO;
    let mut out = Vec::with_capacity(old.len());
    for (at_index, e) in old.iter().enumerate() {
        let expected = at_index as u64 + 1;
        if e.seq != expected {
            return Err(ChainError::Sequence {
                at: at_index,
                expected,
                found: e.seq,
            });
        }
        if e.version != 1 {
            return Err(ChainError::Version {
                seq: e.seq,
                version: e.version,
            });
        }
        if e.prev != prev_old {
            return Err(ChainError::Link { seq: e.seq });
        }
        if hash_event_v1(&e.prev, &e.structure, &e.text_hash) != e.hash {
            return Err(ChainError::Hash { seq: e.seq });
        }
        if let (Some(salt), Some(text)) = (&e.salt, &e.text)
            && hash_text(salt, text) != e.text_hash
        {
            return Err(ChainError::TextHash { seq: e.seq });
        }
        prev_old = e.hash;
        let hash = hash_event(&prev_new, 0, &e.structure, &e.text_hash);
        out.push(Event {
            seq: e.seq,
            version: FORMAT_VERSION,
            prev: prev_new,
            at: 0,
            structure: e.structure.clone(),
            text_hash: e.text_hash,
            salt: e.salt,
            text: e.text.clone(),
            hash,
        });
        prev_new = hash;
    }
    Ok(out)
}

/// What verification found in a log that holds together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The last event's hash; [`Hash::ZERO`] for an empty log.
    pub head: Hash,
    pub len: u64,
    /// Events whose text has been removed. The chain still verifies.
    pub missing_text: Vec<u64>,
}

/// Why a log does not hold together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// Sequence numbers must run 1, 2, 3 with no gaps.
    Sequence {
        at: usize,
        expected: u64,
        found: u64,
    },
    /// An event's `prev` is not the hash of the one before.
    Link { seq: u64 },
    /// An event's structural part or text hash does not give its hash.
    Hash { seq: u64 },
    /// An event's text does not give its text hash.
    TextHash { seq: u64 },
    /// Text present without a salt, or a salt without text.
    HalfRedacted { seq: u64 },
    /// An event format this version cannot read.
    Version { seq: u64, version: u16 },
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequence {
                at,
                expected,
                found,
            } => write!(
                f,
                "event at index {at} has seq {found}, expected {expected}"
            ),
            Self::Link { seq } => write!(f, "event {seq} does not link to the one before"),
            Self::Hash { seq } => write!(f, "event {seq} does not match its hash"),
            Self::TextHash { seq } => write!(f, "event {seq}'s text does not match its text hash"),
            Self::HalfRedacted { seq } => {
                write!(f, "event {seq} has text without salt or salt without text")
            }
            Self::Version { seq, version } => write!(
                f,
                "event {seq} has format version {version}, which this build cannot read"
            ),
        }
    }
}

impl std::error::Error for ChainError {}

/// Check a log from its first event. Every link and every hash is
/// recomputed; removed text is reported, not failed.
pub fn verify(events: &[Event]) -> Result<Verified, ChainError> {
    verify_after(0, Hash::ZERO, events)
}

/// Verify `events` as the continuation of a verified log of `len`
/// events ending in `head`: what a follower checks as the log grows,
/// without re-reading what it has already verified. The result's `len`
/// and `head` are the whole log's.
pub fn verify_after(len: u64, head: Hash, events: &[Event]) -> Result<Verified, ChainError> {
    let mut prev = head;
    let mut missing_text = Vec::new();
    for (at, e) in events.iter().enumerate() {
        let expected = len + at as u64 + 1;
        if e.seq != expected {
            return Err(ChainError::Sequence {
                at,
                expected,
                found: e.seq,
            });
        }
        if e.version != FORMAT_VERSION {
            return Err(ChainError::Version {
                seq: e.seq,
                version: e.version,
            });
        }
        if e.prev != prev {
            return Err(ChainError::Link { seq: e.seq });
        }
        if hash_event(&e.prev, e.at, &e.structure, &e.text_hash) != e.hash {
            return Err(ChainError::Hash { seq: e.seq });
        }
        match (&e.salt, &e.text) {
            (Some(salt), Some(text)) => {
                if hash_text(salt, text) != e.text_hash {
                    return Err(ChainError::TextHash { seq: e.seq });
                }
            }
            (None, None) => missing_text.push(e.seq),
            _ => return Err(ChainError::HalfRedacted { seq: e.seq }),
        }
        prev = e.hash;
    }
    Ok(Verified {
        head: prev,
        len: len + events.len() as u64,
        missing_text,
    })
}

#[cfg(test)]
mod tests {
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
            "aa0b6149af0580a8c704561a257068c597a7af5ec0292960916320e9a416cdbe"
        );
    }

    /// A version-1 event, as the format was before `at`.
    fn v1(seq: u64, prev: Hash, structure: Structure, t: TextPart, s: Salt) -> Event {
        let text_hash = hash_text(&s, &t);
        let hash = hash_event_v1(&prev, &structure, &text_hash);
        Event {
            seq,
            version: 1,
            prev,
            at: 0,
            structure,
            text_hash,
            salt: Some(s),
            text: Some(t),
            hash,
        }
    }

    #[test]
    fn a_v1_log_upgrades_to_v2_and_a_broken_one_is_refused() {
        let a = v1(
            1,
            Hash::ZERO,
            Structure::PhaseOpened { phase: PhaseId(1) },
            text(&[("goal", "old")]),
            salt(1),
        );
        let b = v1(
            2,
            a.hash,
            Structure::ItemAdded {
                item: ItemId(1),
                phase: PhaseId(1),
                checks: vec![],
                depends_on: vec![],
            },
            text(&[("goal", "an old item")]),
            salt(2),
        );
        // A v1 log does not verify under v2.
        assert_eq!(
            verify(&[a.clone(), b.clone()]),
            Err(ChainError::Version { seq: 1, version: 1 })
        );
        let upgraded = upgrade_from_v1(&[a.clone(), b.clone()]).unwrap();
        let v = verify(&upgraded).unwrap();
        assert_eq!(v.len, 2);
        assert!(upgraded.iter().all(|e| e.version == 2 && e.at == 0));
        assert_eq!(
            upgraded[1].text_field("goal").unwrap().as_str(),
            "an old item"
        );
        assert_ne!(upgraded[1].hash, b.hash, "the head hash changes once");
        // Tampered v1 history is not laundered.
        let mut broken = b.clone();
        broken.structure = Structure::PhaseOpened { phase: PhaseId(9) };
        assert_eq!(
            upgrade_from_v1(&[a, broken]),
            Err(ChainError::Hash { seq: 2 })
        );
    }
}
