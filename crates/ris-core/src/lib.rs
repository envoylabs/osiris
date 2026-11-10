//! The pure core of osiris.
//!
//! Everything here is a function of its inputs: a command and the
//! current state give events or a refusal, and state is a fold over the
//! event log. No clock, no filesystem, no database. Those live in the
//! shell that hosts this crate.

pub mod command;
pub mod event;
pub mod ids;
pub mod primitives;
pub mod refusal;
pub mod rules;
pub mod state;
pub mod store;
pub mod text;
pub mod transitions;

pub use command::{Choice, Command, Effect, apply, seal};
pub use event::{Event, RawEvent, Salt, Structure, TextPart, upgrade, verify};
pub use ids::{
    Actor, AgentName, CheckId, ClaimId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
    MessageId, PhaseId,
};
pub use refusal::Refusal;
pub use state::State;
pub use store::{Store, StoreError};
pub use text::{Category, Policy, Text};
