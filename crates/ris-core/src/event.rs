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
//! hash(n)      = H("ris-event-v3" || hash(n-1) || at(n) || structure(n) || text_hash(n))
//! ```
//!
//! The core has no randomness and no clock: the shell supplies the salt
//! and the time (`at`, milliseconds since the Unix epoch; 0 means
//! unknown, as on events upgraded from format version 1). Hashing is
//! SHA-256 over length-prefixed parts, and the canonical form of a part
//! is its compact JSON, so a chain can be checked outside the harness
//! with standard tools.
//!
//! The stored form is the shown form: event names and enum values in
//! snake_case, ids as `I-1`, hashes and salts as hex, actors as
//! `{"kind": "agent", "seat": "heron"}`.
//!
//! Format history: version 1 had no `at`; version 2 added it; version 3
//! changed the JSON form to the one above. [`upgrade`] verifies a
//! version-1 or version-2 log from its stored bytes and rechains it
//! under version 3, recomputing phase report hashes over the new heads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ids::{
    ActionId, Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
    MessageId, PhaseId,
};
use crate::primitives::{Blocker, DecisionClass, MessageKind};
use crate::text::Text;

/// The event format version. Events are versioned from the first
/// release, with an upgrade step for old versions.
pub const FORMAT_VERSION: u16 = 3;

/// Random bytes the shell supplies for each event's text hash. Deleted
/// with the text, so a short text cannot be recovered by guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Salt(pub [u8; 16]);

impl Serialize for Salt {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(
            &self
                .0
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    }
}

impl<'de> Deserialize<'de> for Salt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        crate::ids::decode_hex(&s)
            .and_then(|b| b.try_into().ok())
            .map(Salt)
            .ok_or_else(|| serde::de::Error::custom("expected 32 hex characters"))
    }
}

/// The structural part of an event: everything the fold reads. No free
/// text lives here, by construction: none of these fields is a `Text`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
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
    /// The person replaced a ready or blocked item with another.
    ItemSuperseded {
        item: ItemId,
        by: ItemId,
    },
    /// A person action. The text part carries `what` and `done_when`.
    /// Items it blocks are set aside by `ItemBlocked` events emitted
    /// with it.
    ActionRaised {
        action: ActionId,
        by: Actor,
        blocks: Vec<ItemId>,
        informs: Vec<ItemId>,
    },
    /// The person gave the action a check: a shell command that passes
    /// once the action is done. The text part carries `check`.
    ActionCheckSet {
        action: ActionId,
    },
    /// The person did it. The text part carries `note`, if given.
    ActionDone {
        action: ActionId,
        evidence: Vec<EvidenceId>,
    },
    /// The runner started a seat's agent. The text part carries
    /// `runner` and, when configured, `model`: the binding behind the
    /// seat's name for this run.
    RunStarted {
        seat: AgentName,
    },
    /// The seat's agent stopped, with its exit status when it had one.
    RunStopped {
        seat: AgentName,
        status: Option<i32>,
    },
    /// A seat reported a fault in its setup: something its sandbox or
    /// permissions do not let it do, for the person to fix in the
    /// configuration. The text part carries `what`. The seat's next run
    /// start clears it.
    SetupFault {
        seat: AgentName,
    },
    /// The person paused a seat: its next claim or audit is refused
    /// until it is resumed. What it holds now it finishes.
    SeatPaused {
        seat: AgentName,
    },
    SeatResumed {
        seat: AgentName,
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
    /// The full case for an open decision. The text part carries
    /// `summary`, `why`, and `option.{n}.pro.{k}` and `option.{n}.con.{k}`
    /// for the counts given here, one entry per option.
    DecisionExplained {
        decision: DecisionId,
        by: Actor,
        pros: Vec<usize>,
        cons: Vec<usize>,
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

/// `H("ris-event-v3" || prev || at || canonical(structure) || text_hash)`.
pub fn hash_event(prev: &Hash, at: u64, structure: &Structure, text_hash: &Hash) -> Hash {
    digest(
        "ris-event-v3",
        &[
            &prev.0,
            &at.to_be_bytes(),
            &canonical(structure),
            &text_hash.0,
        ],
    )
}

/// An event as stored by an older format: the structural part as the
/// exact JSON text its hash was computed over, so the old chain can be
/// checked without the old types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEvent {
    pub seq: u64,
    pub version: u16,
    pub prev: Hash,
    pub at: u64,
    pub structure: String,
    pub text_hash: Hash,
    pub salt: Option<Salt>,
    pub text: Option<TextPart>,
    pub hash: Hash,
}

/// The chain hash of an older format, over the structure's stored text.
fn hash_raw(e: &RawEvent) -> Option<Hash> {
    let s = e.structure.as_bytes();
    match e.version {
        1 => Some(digest("ris-event-v1", &[&e.prev.0, s, &e.text_hash.0])),
        2 => Some(digest(
            "ris-event-v2",
            &[&e.prev.0, &e.at.to_be_bytes(), s, &e.text_hash.0],
        )),
        _ => None,
    }
}

/// `PhaseOpened` to `phase_opened`.
fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A version-1 or version-2 structure, as stored, in version 3's form.
/// Field by field, by name; the result must then deserialize as a
/// [`Structure`], which refuses unknown fields.
fn structure_from_old(json: &str) -> Option<Structure> {
    use serde_json::{Value, json};
    let id = |prefix: &str, v: &Value| v.as_u64().map(|n| json!(format!("{prefix}-{n}")));
    let hex = |v: &Value| -> Option<String> {
        let bytes: Option<Vec<u8>> = v
            .as_array()?
            .iter()
            .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
            .collect();
        let bytes = bytes?;
        (bytes.len() == 32).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect())
    };
    let actor = |v: &Value| -> Option<Value> {
        match v {
            Value::String(s) if s == "Person" => Some(json!({ "kind": "person" })),
            Value::Object(o) => Some(json!({ "kind": "agent", "seat": o.get("Agent")?.as_str()? })),
            _ => None,
        }
    };
    let mut v: Value = serde_json::from_str(json).ok()?;
    let o = v.as_object_mut()?;
    let event = o.get("event")?.as_str()?.to_owned();
    for (k, val) in o.iter_mut() {
        let new = match k.as_str() {
            "event" => json!(snake(val.as_str()?)),
            "phase" => id("P", val)?,
            "item" => id("I", val)?,
            "decision" => id("D", val)?,
            "message" => id("M", val)?,
            "about" if val.is_null() => Value::Null,
            "about" => id("I", val)?,
            "depends_on" => Value::Array(
                val.as_array()?
                    .iter()
                    .map(|d| id("I", d))
                    .collect::<Option<_>>()?,
            ),
            "report" => json!(hex(val)?),
            "evidence" if event == "CheckRan" => json!(format!("E-{}", hex(val)?)),
            "evidence" => Value::Array(
                val.as_array()?
                    .iter()
                    .map(|e| hex(e).map(|h| json!(format!("E-{h}"))))
                    .collect::<Option<_>>()?,
            ),
            // ItemAudited's `by` is a seat name, already a string.
            "by" if event == "ItemAudited" => val.clone(),
            "by" | "from" => actor(val)?,
            "class" | "kind" => json!(snake(val.as_str()?)),
            "on" => Value::Array(
                val.as_array()?
                    .iter()
                    .map(|b| {
                        let (kind, n) = b.as_object()?.iter().next()?;
                        let prefix = if kind == "Decision" { "D" } else { "I" };
                        Some(json!({ "kind": snake(kind), "id": id(prefix, n)? }))
                    })
                    .collect::<Option<_>>()?,
            ),
            _ => val.clone(),
        };
        *val = new;
    }
    serde_json::from_value(v).ok()
}

/// Rechain a version-1 or version-2 log under the current format. The
/// old chain is verified first, from the stored bytes, so a broken log
/// is not laundered into a valid one. Timestamps are kept (version 1
/// has none: 0). Each phase report, which is a hash over the previous
/// report and the head before the close, is checked against the old
/// chain and recomputed over the new one. Every hash changes: this is
/// the one-off cost of changing the format.
pub fn upgrade(old: &[RawEvent]) -> Result<Vec<Event>, ChainError> {
    use crate::command::report_hash;
    let (mut prev_old, mut prev_new) = (Hash::ZERO, Hash::ZERO);
    let (mut report_old, mut report_new) = (Hash::ZERO, Hash::ZERO);
    let mut out = Vec::with_capacity(old.len());
    for (index, e) in old.iter().enumerate() {
        let expected = index as u64 + 1;
        if e.seq != expected {
            return Err(ChainError::Sequence {
                at: index,
                expected,
                found: e.seq,
            });
        }
        let Some(hash) = hash_raw(e) else {
            return Err(ChainError::Version {
                seq: e.seq,
                version: e.version,
            });
        };
        if e.prev != prev_old {
            return Err(ChainError::Link { seq: e.seq });
        }
        if hash != e.hash {
            return Err(ChainError::Hash { seq: e.seq });
        }
        match (&e.salt, &e.text) {
            (Some(salt), Some(text)) if hash_text(salt, text) != e.text_hash => {
                return Err(ChainError::TextHash { seq: e.seq });
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(ChainError::HalfRedacted { seq: e.seq });
            }
            _ => {}
        }
        let mut structure =
            structure_from_old(&e.structure).ok_or(ChainError::Unreadable { seq: e.seq })?;
        if let Structure::PhaseClosed { report, .. } = &mut structure {
            if *report != report_hash(&report_old, &e.prev) {
                return Err(ChainError::Report { seq: e.seq });
            }
            report_old = *report;
            report_new = report_hash(&report_new, &prev_new);
            *report = report_new;
        }
        prev_old = e.hash;
        let hash = hash_event(&prev_new, e.at, &structure, &e.text_hash);
        out.push(Event {
            seq: e.seq,
            version: FORMAT_VERSION,
            prev: prev_new,
            at: e.at,
            structure,
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
    /// An old event's structure could not be put in the current form.
    Unreadable { seq: u64 },
    /// A phase report does not match the log it was taken over.
    Report { seq: u64 },
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
            Self::Unreadable { seq } => {
                write!(
                    f,
                    "event {seq}'s structure cannot be read in the current form"
                )
            }
            Self::Report { seq } => {
                write!(
                    f,
                    "event {seq}'s report hash does not match the log before it"
                )
            }
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
#[path = "tests/event.rs"]
mod tests;
