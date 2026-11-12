//! The primitives: the harness's concepts as types.
//!
//! Each primitive's lifecycle is an enum, and the commands that may move
//! it from one state to another are listed in [`crate::transitions`].
//! Illegal moves are refused by the core, and the table is the
//! specification the core is tested against.
//!
//! These records hold no free text. Where a primitive has text (a goal,
//! a question, a rationale, a body) the record carries the sequence
//! number of the event whose text part holds it, as `at`. The fold
//! therefore reads only structure, and the shell joins text back in for
//! display. The exceptions are [`Check`] and [`Evidence`], which come
//! from configuration and disk, not from the log.

use serde::{Deserialize, Serialize};

use crate::ids::{
    ActionId, Actor, AgentName, CheckId, ClaimId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
    MessageId, PhaseId,
};
use crate::text::Text;

/// A phase: a set of items with an exit review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub id: PhaseId,
    /// The event that opened it; its text part holds `goal`.
    pub at: u64,
    pub state: PhaseState,
}

/// Where a phase is in its life.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseState {
    /// Items are being worked. Only one phase is open at a time.
    Open,
    /// The person has reviewed; the report's hash is taken over the
    /// log's head at close.
    Closed { report: Hash },
}

/// An item: one unit of work, and one commit when it merges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    /// The phase it is in now: the one it was added in, or the last one
    /// the person brought it into.
    pub phase: PhaseId,
    /// The phase it was added in, which never changes. A phase's tally
    /// counts its planned and carried-over items by this.
    pub added_in: PhaseId,
    /// The event that added it; its text part holds `goal`.
    pub at: u64,
    /// The checks that must pass for the item to be done, by name.
    pub checks: Vec<CheckId>,
    /// Items this one cannot start before they have merged.
    pub depends_on: Vec<ItemId>,
    /// Every check run against it, in order.
    pub check_runs: Vec<CheckRun>,
    /// The agent that last claimed it: its author once done.
    pub author: Option<AgentName>,
    /// Agents that have reviewed it, in order.
    pub audited_by: Vec<AgentName>,
    pub state: ItemState,
}

/// One run of one check against an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub check: CheckId,
    pub passed: bool,
    pub evidence: EvidenceId,
    pub at: u64,
}

/// Where an item is in its life.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    /// Unclaimed and unblocked.
    Ready,
    /// One agent holds it. Leases come with the daemon.
    Claimed { agent: AgentName },
    /// Set aside until the blockers clear: a substantive decision, or an
    /// item it depends on.
    Blocked { on: Vec<Blocker> },
    /// Its checks passed; the evidence is attached.
    Done { evidence: Vec<EvidenceId> },
    /// Its commit is on the phase's branch. Changed only by a further
    /// item.
    Merged {
        evidence: Vec<EvidenceId>,
        commit: CommitHash,
    },
    /// Replaced by another item before work on it began; final, as a
    /// superseded decision is. A dependency on it is a dependency on
    /// its replacement.
    Superseded { by: ItemId },
}

/// What an item can be blocked on.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Blocker {
    Decision(DecisionId),
    Item(ItemId),
    Action(ActionId),
}

/// Something only the person can do: run a trial, provide a
/// prerequisite, do a chore. Raised and done only by the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub by: Actor,
    /// The event that raised it; its text part holds `what` and
    /// `done_when`.
    pub at: u64,
    /// Items that wait for it.
    pub blocks: Vec<ItemId>,
    /// Items that proceed without it and are shaped by it.
    pub informs: Vec<ItemId>,
    /// The event that gave it a check, whose text part holds `check`.
    pub check: Option<u64>,
    pub state: ActionState,
}

/// Where an action is in its life.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionState {
    Open,
    /// The event that recorded it holds `note`, if one was given.
    Done {
        at: u64,
        evidence: Vec<EvidenceId>,
    },
}

/// A decision: a question with options and a recommendation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub class: DecisionClass,
    /// Who raised it.
    pub by: Actor,
    /// The event that raised it; its text part holds `question` and
    /// `option.0` to `option.{options - 1}`.
    pub at: u64,
    pub options: usize,
    /// Index into the options.
    pub recommended: usize,
    /// The item that raised it, if one did.
    pub about: Option<ItemId>,
    pub state: DecisionState,
    /// The latest explanation: its event, and how many pros and cons
    /// it gives each option.
    pub explained: Option<Explanation>,
}

/// Where a decision's explanation is: the event whose text part holds
/// it, and the shape of that text part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Explanation {
    pub at: u64,
    pub by: Actor,
    pub pros: Vec<usize>,
    pub cons: Vec<usize>,
}

/// The decision rule: the test is the cost of reversing.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionClass {
    /// The agent goes ahead and logs it.
    Mechanical,
    /// Expensive to reverse: queued for the person.
    Substantive,
}

/// Where a decision is in its life.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionState {
    Open,
    Taken {
        by: Actor,
        /// Index into the decision's options.
        chosen: usize,
        /// The event that took it; its text part holds `rationale`.
        at: u64,
    },
    /// Replaced by a later decision; never edited in place.
    Superseded(DecisionId),
}

/// A check: a command and its pass condition, declared per project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: CheckId,
    pub command: Text,
    pub pass: PassCondition,
}

/// What counts as a check passing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassCondition {
    /// The command exits with status zero.
    ExitZero,
}

/// Evidence: an artefact attached to an item or decision, held on disk
/// by content hash. The store holds only this record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: EvidenceId,
    /// What produced it: a check's run, or a file the actor attached.
    pub source: EvidenceSource,
    pub bytes: u64,
}

/// Where a piece of evidence came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSource {
    CheckRun { check: CheckId, passed: bool },
    Attached { label: Text },
}

/// A claim: a statement the harness holds as true, with provenance, a
/// falsifier, and a state that says how far it is trusted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub id: ClaimId,
    /// The event that asserted it; its text part holds `statement`.
    pub at: u64,
    pub source: Provenance,
    /// The check that would show it false.
    pub falsifier: CheckId,
    pub state: ClaimState,
}

/// Where a claim came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub item: Option<ItemId>,
    pub decision: Option<DecisionId>,
    pub commit: Option<CommitHash>,
}

/// How far a claim is trusted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimState {
    /// Trusted in the moment.
    Asserted,
    /// Settled at review, evidence attached, falsifier passing.
    Final {
        evidence: Vec<EvidenceId>,
        at: CommitHash,
    },
    /// Its falsifier failed; reopened as an item.
    Falsified { by: EvidenceId },
    /// Replaced by an explicit decision.
    Superseded(DecisionId),
}

/// A message: a typed, targeted note between agents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub from: Actor,
    pub to: AgentName,
    pub kind: MessageKind,
    pub about: Option<ItemId>,
    /// The event that sent it; its text part holds `body`.
    pub at: u64,
}

/// What a message is for. No broadcasts and no free chat.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Work passed from one agent to another.
    Handoff,
    /// Something a dependent item relies on changed.
    InterfaceChanged,
    /// A question to an item's owner.
    Question,
    /// An auditor's finding on an item.
    Finding,
}
