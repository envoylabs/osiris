//! The pure core of osiris.
//!
//! Everything here is a function of its inputs: a command and the
//! current state give events or a refusal, and state is a fold over the
//! event log. No clock, no filesystem, no database. Those live in the
//! shell that hosts this crate.

pub mod event;
pub mod ids;
pub mod primitives;
pub mod refusal;
pub mod text;
pub mod transitions;

pub use event::{Event, Salt, Structure, TextPart, verify};
pub use ids::{
    Actor, AgentName, CheckId, ClaimId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
    MessageId, PhaseId,
};
pub use refusal::Refusal;
pub use text::{Category, Policy, Text};
