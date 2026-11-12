//! The `--json` contract as types, and the JSON Schema generated from
//! them.
//!
//! Every command's `data` is one of the types here (or a core type),
//! and [`Output::of`](crate::render::Output::of) takes nothing else, so
//! what a command prints and what the schema says of it come from the
//! same declaration and cannot drift. [`document`] assembles the schema
//! for format 1: the envelope, the error, and each command's data under
//! `$defs` as `data.<command>`, with spaces in a command written `_`
//! (`data.phase_open`). `ris schema` prints it, and it is committed at
//! [`PATH`]; a test regenerates it and fails when the file differs.
//!
//! The schema describes what `ris` writes. Within a format, keys are
//! added and never removed, renamed or reshaped, so the objects here
//! are open: a client validating against an older copy of the schema
//! accepts a document from a newer `ris` of the same format.

use ris_core::event::Event;
use ris_core::ids::{
    ActionId, Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
    MessageId, PhaseId,
};
use ris_core::primitives::{Blocker, DecisionClass};
use ris_core::refusal::Refusal;
use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use serde_json::{Value, json};

use crate::config::Role;
use crate::render::JSON_FORMAT;

/// Where the schema is committed, relative to the repository root.
pub const PATH: &str = "docs/json.schema.json";

// ---- records shared by several commands ----

/// An item.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Item {
    pub id: ItemId,
    pub phase: PhaseId,
    pub goal: String,
    /// `ready`, `claimed`, `blocked`, `done`, `merged` or `superseded`.
    pub state: &'static str,
    pub checks: Vec<CheckId>,
    pub depends_on: Vec<ItemId>,
    pub audited_by: Vec<AgentName>,
    /// The seat holding it, when claimed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
    /// What it waits on, when blocked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_on: Option<Vec<Blocker>>,
    /// Its evidence, when done or merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceId>>,
    /// Its commit, when merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitHash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<ItemId>,
    /// The unmerged items a ready item depends on, when there are any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<Vec<ItemId>>,
    /// The decisions about it; in `ris show I-n` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<Decision>>,
    /// Each run of one of its checks; in `ris show I-n` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_runs: Option<Vec<CheckRun>>,
}

/// One run of a check against an item.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CheckRun {
    pub check: CheckId,
    pub passed: bool,
    pub evidence: EvidenceId,
}

/// A decision.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Decision {
    pub id: DecisionId,
    pub class: DecisionClass,
    pub by: Actor,
    pub about: Option<ItemId>,
    pub question: String,
    pub options: Vec<String>,
    /// The recommended option's number.
    pub recommended: usize,
    /// `open`, `taken` or `superseded`.
    pub state: &'static str,
    /// Whether it has a full case (`ris eli5`).
    pub explained: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taken_by: Option<Actor>,
    /// The option taken; one past the offered options for an answer
    /// outside them, which is then in `other`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// The sequence number of the event that raised it; in
    /// `ris show D-n` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised_at_event: Option<u64>,
}

/// A person action.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Action {
    pub id: ActionId,
    pub by: Actor,
    pub what: String,
    pub done_when: String,
    pub blocks: Vec<ItemId>,
    pub informs: Vec<ItemId>,
    /// `open` or `done`.
    pub state: &'static str,
    /// The command that closes it when it passes, if it has one.
    pub check: Option<String>,
    /// Present once done; null when it was closed without a note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceId>>,
}

/// A message to a seat.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Message {
    pub id: MessageId,
    pub from: Actor,
    pub to: AgentName,
    /// `handoff`, `interfacechanged`, `question` or `finding`.
    pub kind: String,
    pub about: Option<ItemId>,
    pub body: String,
}

/// A decision's full case.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Case {
    pub decision: Decision,
    /// Null when no explanation is recorded.
    pub explanation: Option<Explanation>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Explanation {
    pub by: Actor,
    pub summary: String,
    pub options: Vec<ExplainedOption>,
    /// Why the recommendation is the recommendation.
    pub why: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExplainedOption {
    pub option: usize,
    pub text: String,
    pub pros: Vec<String>,
    pub cons: Vec<String>,
}

/// What waits on what.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// How many nodes have no edge.
    pub unlinked: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GraphNode {
    /// An item's, decision's or action's id.
    pub id: String,
    pub state: &'static str,
    /// An item's phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<PhaseId>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// `blocks`, `informs` or `met`.
    pub kind: &'static str,
}

// ---- each command's data ----

/// `ris ready`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Ready {
    pub ready: Vec<Item>,
}

/// `ris inbox`: the person's, or a seat's.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Inbox {
    Person {
        decisions: Vec<Decision>,
        actions: Vec<Action>,
        /// What running the person's action checks did; in the
        /// command's own data, not where an inbox is part of another.
        #[serde(skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<String>>,
    },
    Seat {
        holding: Vec<Item>,
        messages: Vec<Message>,
        #[serde(skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<String>>,
    },
}

/// `ris report`: the phase report, or that there is no phase yet.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Report {
    Phase(Box<PhaseReport>),
    NoPhase {
        /// Null: no phase has been opened.
        phase: (),
    },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PhaseReport {
    pub phase: PhaseHead,
    pub items: Vec<Item>,
    pub tally: crate::render::Tally,
    pub substantive: Vec<Decision>,
    pub mechanical: Vec<Decision>,
    /// How many messages the log holds.
    pub messages: usize,
    pub answered_outside_options: usize,
    pub merged_ahead: Vec<MergedAhead>,
    pub log: LogHead,
    pub report: Hash,
    pub previous_report: Hash,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PhaseHead {
    pub id: PhaseId,
    pub goal: String,
    /// `open` or `closed`.
    pub state: &'static str,
}

/// An item merged before a person action that informs it was done.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MergedAhead {
    pub item: ItemId,
    pub action: ActionId,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LogHead {
    pub len: u64,
    pub head: Hash,
}

/// `ris summary`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Summary {
    pub inbox: Inbox,
    pub graph: Graph,
    pub report: Report,
    pub ready: Ready,
    /// Whether `--all` was given.
    pub all: bool,
    pub carried_over: CarriedOver,
    /// What running the person's action checks did.
    pub checks: Vec<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CarriedOver {
    pub items: usize,
    pub decisions: usize,
}

/// `ris show <id>`: the record the id names.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Show {
    Item(Box<Item>),
    Decision(Box<Decision>),
    Action(Action),
    Phase(Phase),
    Message(Message),
    Evidence(EvidenceContent),
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Phase {
    pub id: PhaseId,
    pub goal: String,
    pub state: &'static str,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EvidenceContent {
    /// `E-` and the hash, in full.
    pub evidence: String,
    pub content: String,
}

/// `ris audit` with no item.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AuditQueue {
    pub queue: Vec<Item>,
}

/// `ris audit I-n`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Audited {
    pub item: ItemId,
    pub by: AgentName,
}

/// `ris audit`: the queue, or the review recorded.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Audit {
    Queue(AuditQueue),
    Recorded(Audited),
}

/// `ris claim`: the brief.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Brief {
    pub rules: BriefRules,
    pub project: BriefProject,
    pub item: ItemId,
    pub goal: String,
    pub checks: Vec<CheckId>,
    pub depends_on: Vec<BriefDependency>,
    pub actions: Vec<BriefAction>,
    pub decisions: Vec<BriefDecision>,
    /// Older decisions the brief had no room for.
    pub decisions_left_out: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BriefRules {
    /// Relative to the project root; null when the built-in rules are
    /// meant.
    pub path: Option<String>,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BriefProject {
    pub phase: PhaseId,
    pub goal: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BriefDependency {
    pub item: ItemId,
    pub state: &'static str,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BriefAction {
    pub id: ActionId,
    /// `blocks` or `informs`.
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BriefDecision {
    pub id: DecisionId,
    pub state: &'static str,
}

/// A command that names one item.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ItemRef {
    pub item: ItemId,
}

/// `ris item supersede`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Superseded {
    pub item: ItemId,
    pub superseded_by: ItemId,
}

/// `ris done`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Done {
    pub item: ItemId,
    pub evidence: Vec<EvidenceId>,
}

/// `ris merge`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Merged {
    pub item: ItemId,
    pub commit: String,
}

/// `ris check`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Checks {
    pub checks: Vec<CheckResult>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CheckResult {
    pub check: String,
    pub passed: bool,
}

/// `ris decision raise` and `ris decision explain`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DecisionRef {
    pub decision: DecisionId,
}

/// `ris decide`: a named decision taken, the walk's outcome, or with
/// `--json` and no decision the queue.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Decide {
    Taken {
        decision: DecisionId,
        /// Items the decision alone had set aside, ready again.
        unblocked: Vec<ItemId>,
    },
    Walk {
        taken: Vec<DecisionId>,
        open: Vec<DecisionId>,
    },
    Queue {
        queue: Vec<Case>,
    },
}

/// `ris action raise`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActionRaised {
    pub action: ActionId,
    pub blocked: Vec<String>,
}

/// `ris action done`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActionDone {
    pub action: ActionId,
    pub ready: Vec<ItemId>,
}

/// `ris action check`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActionChecked {
    pub action: ActionId,
    pub checks: Vec<String>,
}

/// `ris send`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Sent {
    pub message: MessageId,
}

/// `ris init`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Init {
    pub root: String,
    pub local: bool,
}

/// `ris rules`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Rules {
    pub rules: String,
}

/// `ris worktree`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Worktree {
    pub seat: String,
    pub path: String,
}

/// A command that names one seat.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatRef {
    pub seat: String,
}

/// `ris seat add`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatAdded {
    pub seat: String,
    pub role: Role,
}

/// `ris seat start`: the run it waited for, once it has stopped.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatStopped {
    pub seat: String,
    /// The agent's exit status; null when a signal ended it.
    pub status: Option<i32>,
    pub log: String,
}

/// `ris seat pause` and `ris seat resume`: one of `paused` and
/// `resumed`, the seats named.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatsSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed: Option<Vec<String>>,
    /// What each paused seat holding an item finishes first.
    pub finishing: Vec<String>,
}

/// `ris up`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Up {
    pub changed: Vec<String>,
    pub not_in_file: Vec<String>,
    pub attended: Vec<String>,
}

/// `ris down`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Down {
    pub changed: Vec<String>,
    pub attended: Vec<String>,
}

/// `ris install`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Installed {
    pub item: ItemId,
    pub commit: String,
    pub to: String,
}

/// `ris logs`: one seat's lines, every seat's, or when followed one
/// line per envelope.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Logs {
    Seat { seat: String, lines: Vec<String> },
    All { lines: Vec<SeatLine> },
    Line(SeatLine),
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatLine {
    pub seat: String,
    pub line: String,
}

/// `ris phase open`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PhaseOpened {
    pub phase: PhaseId,
    /// The phase's branch; null when phase branches are off.
    pub branch: Option<String>,
    /// The item added to bring the docs up to date, if one was.
    pub docs_item: Option<ItemId>,
    /// What the writing seats would lack to run the checks.
    pub preflight: Vec<crate::preflight::Missing>,
}

/// `ris phase close`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PhaseClosed {
    /// The report's hash.
    pub report: Hash,
    pub phase: Report,
    /// What the closing merge did, in words.
    pub closing_merge: String,
}

/// `ris status`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Status {
    pub seats: Vec<SeatStatus>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeatStatus {
    pub seat: String,
    pub role: Role,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub claim: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub uncommitted: Option<u32>,
    pub paused: bool,
    pub running: bool,
    pub activity: Option<crate::status::Activity>,
    pub faults: Vec<String>,
}

/// `ris scan`, clean. A hit is the `leak` error instead.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Scanned {
    pub files: usize,
    /// Always empty.
    pub hits: Vec<Hit>,
}

/// `ris verify`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Verified {
    pub len: u64,
    pub head: Hash,
    /// The sequence numbers of events whose text has been removed.
    pub missing_text: Vec<u64>,
}

/// `ris import`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Imported {
    pub items: usize,
    pub decisions: usize,
    /// How many of `decisions` are open.
    pub open: usize,
    /// False for a dry run.
    pub written: bool,
}

/// `ris export`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Exported {
    pub phases: Vec<String>,
    pub new: usize,
}

/// `ris map`: the graph document (`docs/json.md`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Map {
    pub phase: Option<PhaseId>,
    pub base: Option<String>,
    pub tip: String,
    pub tip_ref: String,
    pub totals: MapTotals,
    pub nodes: Vec<crate::map::Node>,
    pub edges: Vec<crate::map::Edge>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MapTotals {
    pub files: usize,
    pub changed: usize,
    pub added: u64,
    pub removed: u64,
    pub outside: usize,
    /// The test files left off the map.
    pub tests: MapTests,
}

/// The test files `[map] tests` names, which are not nodes: how many
/// there are, how many the phase changed, and the lines it added and
/// removed in them.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct MapTests {
    pub files: usize,
    pub changed: usize,
    pub added: u64,
    pub removed: u64,
}

// ---- the error ----

/// A failure: `error` in the envelope.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    /// The suggested next step.
    pub next: Option<String>,
    /// `refused`: the core's reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    /// `leak`: what was scanned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned: Option<String>,
    /// `leak`: where, and never the matched text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hits: Option<Vec<Hit>>,
    /// `checks_failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<Vec<FailedCheck>>,
    /// `role_forbids`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Refused,
    Leak,
    RoleForbids,
    CheckFailed,
    ChecksFailed,
    NotAProject,
    AlreadyAProject,
    Config,
    UnknownAgent,
    NoActor,
    Contended,
    Store,
    Io,
    Other,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Hit {
    pub file: String,
    /// Null when the hit is not on a line of the file (its name, say).
    pub line: Option<usize>,
    /// What kind of thing the scan found, in words.
    pub category: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FailedCheck {
    pub check: String,
    pub evidence: String,
}

// ---- the document ----

/// Every command, as its subcommand path, with the schema of its
/// `data`. A stream's entry is the data of one line. A command not
/// listed here has no schema, which a test refuses.
pub fn commands(g: &mut SchemaGenerator) -> Vec<(&'static str, Schema)> {
    vec![
        ("init", g.subschema_for::<Init>()),
        ("worktree", g.subschema_for::<Worktree>()),
        ("up", g.subschema_for::<Up>()),
        ("down", g.subschema_for::<Down>()),
        ("logs", g.subschema_for::<Logs>()),
        ("rules", g.subschema_for::<Rules>()),
        ("seat add", g.subschema_for::<SeatAdded>()),
        ("seat start", g.subschema_for::<SeatStopped>()),
        ("seat stopped", g.subschema_for::<SeatRef>()),
        ("seat pause", g.subschema_for::<SeatsSet>()),
        ("seat resume", g.subschema_for::<SeatsSet>()),
        ("seat fault", g.subschema_for::<SeatRef>()),
        ("phase open", g.subschema_for::<PhaseOpened>()),
        ("phase close", g.subschema_for::<PhaseClosed>()),
        ("item add", g.subschema_for::<ItemRef>()),
        ("item supersede", g.subschema_for::<Superseded>()),
        ("ready", g.subschema_for::<Ready>()),
        ("claim", g.subschema_for::<Brief>()),
        ("release", g.subschema_for::<ItemRef>()),
        ("audit", g.subschema_for::<Audit>()),
        ("action raise", g.subschema_for::<ActionRaised>()),
        ("action check", g.subschema_for::<ActionChecked>()),
        ("action done", g.subschema_for::<ActionDone>()),
        ("decision raise", g.subschema_for::<DecisionRef>()),
        ("decision explain", g.subschema_for::<DecisionRef>()),
        ("eli5", g.subschema_for::<Case>()),
        ("decide", g.subschema_for::<Decide>()),
        ("check", g.subschema_for::<Checks>()),
        ("done", g.subschema_for::<Done>()),
        ("merge", g.subschema_for::<Merged>()),
        ("inbox", g.subschema_for::<Inbox>()),
        ("send", g.subschema_for::<Sent>()),
        ("show", g.subschema_for::<Show>()),
        ("summary", g.subschema_for::<Summary>()),
        ("report", g.subschema_for::<Report>()),
        ("map", g.subschema_for::<Map>()),
        ("status", g.subschema_for::<Status>()),
        ("scan", g.subschema_for::<Scanned>()),
        ("verify", g.subschema_for::<Verified>()),
        (
            "schema",
            json_schema!({
                "type": "object",
                "description": "This document: the JSON Schema of format 1."
            }),
        ),
        ("install", g.subschema_for::<Installed>()),
        ("import", g.subschema_for::<Imported>()),
        ("export", g.subschema_for::<Exported>()),
        ("log", g.subschema_for::<Event>()),
        ("watch", g.subschema_for::<Event>()),
    ]
}

/// The name under `$defs` of a command's data: `data.` and the command
/// with `_` for each space.
pub fn data_name(command: &str) -> String {
    format!("data.{}", command.replace(' ', "_"))
}

/// The JSON Schema of the `--json` contract, format 1.
pub fn document() -> Value {
    let mut g = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    let commands = commands(&mut g);
    let error = g.subschema_for::<Error>();
    let mut defs = g.take_definitions(true);
    let mut any = Vec::new();
    for (command, schema) in commands {
        let name = data_name(command);
        any.push(json!({ "$ref": format!("#/$defs/{name}") }));
        defs.insert(name, schema.to_value());
    }
    defs.insert(
        "envelope.success".to_owned(),
        json!({
            "description": "A successful outcome. `data` is the command's own: `#/$defs/data.<command>`.",
            "type": "object",
            "properties": {
                "format": { "const": JSON_FORMAT },
                "ok": { "const": true },
                "data": { "anyOf": any },
            },
            "required": ["format", "ok", "data"],
        }),
    );
    defs.insert(
        "envelope.failure".to_owned(),
        json!({
            "description": "A failure. The exit status is not zero.",
            "type": "object",
            "properties": {
                "format": { "const": JSON_FORMAT },
                "ok": { "const": false },
                "error": error,
            },
            "required": ["format", "ok", "error"],
        }),
    );
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("ris --json, format {JSON_FORMAT}"),
        "description": "Every outcome of every ris command run with --json is one envelope \
                        on stdout; a stream prints one envelope per line. The data of each \
                        command is under $defs as data.<command>, with _ for a space in the \
                        command. Objects are open: within a format keys are added, never \
                        removed, renamed or reshaped.",
        "oneOf": [
            { "$ref": "#/$defs/envelope.success" },
            { "$ref": "#/$defs/envelope.failure" },
        ],
        "$defs": defs,
    })
}

/// The document as it is committed and printed: indented, with a final
/// newline.
pub fn text() -> String {
    let mut s = serde_json::to_string_pretty(&document()).expect("a JSON value");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_one_entry_and_every_reference_resolves() {
        let mut g = SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator();
        let names: Vec<&str> = commands(&mut g).into_iter().map(|(n, _)| n).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "a command is listed twice");

        let doc = document();
        let defs = doc["$defs"].as_object().unwrap();
        for name in &names {
            assert!(defs.contains_key(&data_name(name)), "{name}");
        }
        // Every $ref in the document points at a definition in it.
        fn refs(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    if let Some(Value::String(r)) = m.get("$ref") {
                        out.push(r.clone());
                    }
                    m.values().for_each(|v| refs(v, out));
                }
                Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        refs(&doc, &mut found);
        assert!(!found.is_empty());
        for r in found {
            let name = r.strip_prefix("#/$defs/").unwrap_or_else(|| panic!("{r}"));
            assert!(defs.contains_key(name), "{r} does not resolve");
        }
    }

    #[test]
    fn the_document_is_the_same_each_time_it_is_generated() {
        assert_eq!(text(), text());
    }

    /// The committed file is what the types generate: a change to any
    /// output type that is not regenerated and committed fails here.
    #[test]
    fn the_committed_schema_is_the_generated_one() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(PATH);
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == text(),
            "{PATH} is not what the output types generate\n  \
             next: `cargo run -p ris -- schema > {PATH}`, read the diff, and commit it; \
             a key removed, renamed or reshaped there is a new format, not format {JSON_FORMAT}"
        );
    }
}
