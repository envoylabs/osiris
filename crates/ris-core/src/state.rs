//! State: a fold over the event log.
//!
//! [`State::fold`] reads each event's structural part and nothing else.
//! The text part is not available to it by type, so replaying a log
//! with every text removed gives the same state as replaying it whole.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::{Event, Structure};
use crate::ids::{DecisionId, Hash, ItemId, MessageId, PhaseId};
use crate::primitives::{
    Blocker, CheckRun, Decision, DecisionState, Item, ItemState, Message, Phase, PhaseState,
};

/// Everything the log says, as of its last event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Number of events folded; the next event's `seq` is `len + 1`.
    pub len: u64,
    /// The last event's hash; [`Hash::ZERO`] when empty.
    pub head: Hash,
    pub phases: BTreeMap<PhaseId, Phase>,
    pub items: BTreeMap<ItemId, Item>,
    pub decisions: BTreeMap<DecisionId, Decision>,
    pub messages: Vec<Message>,
    /// The last closed phase's report hash; [`Hash::ZERO`] before any.
    pub last_report: Hash,
    /// The links read backwards, kept by the fold.
    pub index: Index,
    /// Seats the person has paused.
    pub paused: std::collections::BTreeSet<crate::ids::AgentName>,
    pub actions: BTreeMap<crate::ids::ActionId, crate::primitives::Action>,
    /// Seats whose agent the runner started and has not seen stop, with
    /// the event that started it.
    pub running: BTreeMap<crate::ids::AgentName, u64>,
    /// Setup faults each seat reported since its latest run started, as
    /// the events that recorded them, oldest first.
    #[serde(default)]
    pub faults: BTreeMap<crate::ids::AgentName, Vec<u64>>,
}

/// The reverse index: links that events record in one direction, read
/// in the other. Derived, like the rest of the state, and rebuilt on
/// every fold; [`State::check_index`] compares it with a full scan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// For each item, the items that depend on it, in the order added.
    pub dependents: BTreeMap<ItemId, Vec<ItemId>>,
    /// For each item, the decisions raised about it, in the order raised.
    pub decisions_about: BTreeMap<ItemId, Vec<DecisionId>>,
    /// For each item, the items it directly replaced, in id order.
    pub replaced: BTreeMap<ItemId, Vec<ItemId>>,
    /// For each item, the person actions that block or inform it, in
    /// the order raised.
    pub actions_about: BTreeMap<ItemId, Vec<crate::ids::ActionId>>,
}

/// The index disagrees with a scan of the items and decisions: a bug in
/// the fold, never a fact about the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexMismatch {
    pub kept: Index,
    pub scanned: Index,
}

impl std::fmt::Display for IndexMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the reverse index disagrees with a scan of the state; this is a bug in ris")
    }
}

impl std::error::Error for IndexMismatch {}

/// A structural reference that the log should have made impossible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldError {
    UnknownItem {
        seq: u64,
        item: ItemId,
    },
    UnknownDecision {
        seq: u64,
        decision: DecisionId,
    },
    UnknownPhase {
        seq: u64,
        phase: PhaseId,
    },
    UnknownAction {
        seq: u64,
        action: crate::ids::ActionId,
    },
    /// An event this version of ris records in the format but does not
    /// yet fold: an `ItemScoped` with no phase.
    Unsupported {
        seq: u64,
    },
}

impl std::fmt::Display for FoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownItem { seq, item } => write!(f, "event {seq} refers to unknown {item}"),
            Self::UnknownDecision { seq, decision } => {
                write!(f, "event {seq} refers to unknown {decision}")
            }
            Self::UnknownPhase { seq, phase } => write!(f, "event {seq} refers to unknown {phase}"),
            Self::UnknownAction { seq, action } => {
                write!(f, "event {seq} refers to unknown {action}")
            }
            Self::Unsupported { seq } => {
                write!(f, "event {seq} is not supported by this version of ris")
            }
        }
    }
}

impl std::error::Error for FoldError {}

impl State {
    /// The state after every event in `log`, from empty.
    pub fn fold(log: &[Event]) -> Result<Self, FoldError> {
        let mut state = Self::default();
        for e in log {
            state.apply_event(e)?;
        }
        debug_assert_eq!(state.check_index(), Ok(()));
        Ok(state)
    }

    /// The phase that is open, if one is.
    pub fn open_phase(&self) -> Option<&Phase> {
        self.phases
            .values()
            .find(|p| matches!(p.state, PhaseState::Open))
    }

    /// Items an agent may claim now: ready, with every dependency merged.
    pub fn ready(&self) -> impl Iterator<Item = &Item> {
        self.items.values().filter(|i| {
            matches!(i.state, ItemState::Ready) && self.unlanded_dependencies(i).is_empty()
        })
    }

    /// Dependencies of `item` that have not merged. A superseded
    /// dependency stands for its replacement.
    pub fn unlanded_dependencies(&self, item: &Item) -> Vec<ItemId> {
        item.depends_on
            .iter()
            .map(|d| self.current(*d))
            .filter(|d| {
                !matches!(
                    self.items.get(d).map(|i| &i.state),
                    Some(ItemState::Merged { .. })
                )
            })
            .collect()
    }

    /// Whether superseding `item` by `by` would make `by` wait on
    /// itself: its dependencies, followed through replacements with
    /// `item` standing for `by`, reach `by` again. Items are added
    /// depending only on items that exist, so a supersede is the only
    /// way a cycle forms, and every cycle it forms passes through `by`.
    pub fn would_wait_on_itself(&self, item: ItemId, by: ItemId) -> bool {
        let after = |d: ItemId| {
            let c = self.current(d);
            if c == item { by } else { c }
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![by];
        while let Some(i) = stack.pop() {
            let Some(it) = self.items.get(&i) else {
                continue;
            };
            for d in &it.depends_on {
                let d = after(*d);
                if d == by {
                    return true;
                }
                if seen.insert(d) {
                    stack.push(d);
                }
            }
        }
        false
    }

    /// The item that stands for `item` now: itself, or the end of its
    /// chain of replacements. The chain cannot loop: an item is only
    /// superseded by one that is not itself superseded.
    pub fn current(&self, mut item: ItemId) -> ItemId {
        while let Some(ItemState::Superseded { by }) = self.items.get(&item).map(|i| &i.state) {
            item = *by;
        }
        item
    }

    /// The index as a scan of the items and decisions gives it.
    pub fn scan_index(&self) -> Index {
        let mut ix = Index::default();
        for i in self.items.values() {
            for d in &i.depends_on {
                ix.dependents.entry(*d).or_default().push(i.id);
            }
            if let ItemState::Superseded { by } = i.state {
                ix.replaced.entry(by).or_default().push(i.id);
            }
        }
        for d in self.decisions.values() {
            if let Some(a) = d.about {
                ix.decisions_about.entry(a).or_default().push(d.id);
            }
        }
        for a in self.actions.values() {
            for i in a.blocks.iter().chain(&a.informs) {
                ix.actions_about.entry(*i).or_default().push(a.id);
            }
        }
        ix
    }

    /// Whether the kept index matches a scan.
    pub fn check_index(&self) -> Result<(), Box<IndexMismatch>> {
        let scanned = self.scan_index();
        if scanned == self.index {
            Ok(())
        } else {
            Err(Box::new(IndexMismatch {
                kept: self.index.clone(),
                scanned,
            }))
        }
    }

    /// Items that depend on `item`, in the order added.
    pub fn dependents(&self, item: ItemId) -> &[ItemId] {
        self.index.dependents.get(&item).map_or(&[], Vec::as_slice)
    }

    /// Decisions raised about `item`, in the order raised.
    pub fn decisions_about(&self, item: ItemId) -> &[DecisionId] {
        self.index
            .decisions_about
            .get(&item)
            .map_or(&[], Vec::as_slice)
    }

    /// `item` and every item it stands for: those it replaced, and
    /// those they replaced, in no particular order.
    pub fn lineage(&self, item: ItemId) -> Vec<ItemId> {
        let mut out = vec![item];
        let mut n = 0;
        while n < out.len() {
            if let Some(r) = self.index.replaced.get(&out[n]) {
                out.extend(r);
            }
            n += 1;
        }
        out
    }

    /// Open decisions, in the order raised.
    pub fn open_decisions(&self) -> impl Iterator<Item = &Decision> {
        self.decisions
            .values()
            .filter(|d| matches!(d.state, DecisionState::Open))
    }

    pub fn next_phase_id(&self) -> PhaseId {
        PhaseId(self.phases.len() as u32 + 1)
    }

    pub fn next_item_id(&self) -> ItemId {
        ItemId(self.items.len() as u32 + 1)
    }

    pub fn next_decision_id(&self) -> DecisionId {
        DecisionId(self.decisions.len() as u32 + 1)
    }

    pub fn next_action_id(&self) -> crate::ids::ActionId {
        crate::ids::ActionId(self.actions.len() as u32 + 1)
    }

    /// Person actions that block or inform `item`, in the order raised.
    pub fn actions_about(&self, item: ItemId) -> &[crate::ids::ActionId] {
        self.index
            .actions_about
            .get(&item)
            .map_or(&[], Vec::as_slice)
    }

    pub fn next_message_id(&self) -> MessageId {
        MessageId(self.messages.len() as u32 + 1)
    }

    /// Fold one event. Reads `e.structure`, `e.seq` and `e.hash` only.
    pub fn apply_event(&mut self, e: &Event) -> Result<(), FoldError> {
        let seq = e.seq;
        match &e.structure {
            Structure::PhaseOpened { phase } => {
                self.phases.insert(
                    *phase,
                    Phase {
                        id: *phase,
                        at: seq,
                        state: PhaseState::Open,
                    },
                );
            }
            Structure::PhaseClosed { phase, report } => {
                let p = self
                    .phases
                    .get_mut(phase)
                    .ok_or(FoldError::UnknownPhase { seq, phase: *phase })?;
                p.state = PhaseState::Closed { report: *report };
                self.last_report = *report;
            }
            Structure::ItemAdded {
                item,
                phase,
                checks,
                depends_on,
            } => {
                self.items.insert(
                    *item,
                    Item {
                        id: *item,
                        phase: *phase,
                        added_in: *phase,
                        at: seq,
                        checks: checks.clone(),
                        depends_on: depends_on.clone(),
                        check_runs: Vec::new(),
                        author: None,
                        audited_by: Vec::new(),
                        state: ItemState::Ready,
                    },
                );
                for d in depends_on {
                    self.index.dependents.entry(*d).or_default().push(*item);
                }
            }
            Structure::ItemScoped { item, phase } => {
                let Some(phase) = phase else {
                    // D-56 gives the event a null phase for an item
                    // deferred to the backlog. The command that writes
                    // it, and how state shows an item with no phase,
                    // come with deferral; until then no ris writes one.
                    return Err(FoldError::Unsupported { seq });
                };
                if !self.phases.contains_key(phase) {
                    return Err(FoldError::UnknownPhase { seq, phase: *phase });
                }
                self.item_mut(seq, *item)?.phase = *phase;
            }
            Structure::ItemClaimed { item, agent } => {
                let i = self.item_mut(seq, *item)?;
                i.author = Some(agent.clone());
                i.state = ItemState::Claimed {
                    agent: agent.clone(),
                };
            }
            Structure::ItemAudited { item, by } => {
                self.item_mut(seq, *item)?.audited_by.push(by.clone());
            }
            Structure::ItemBlocked { item, on } => {
                self.item_mut(seq, *item)?.state = ItemState::Blocked { on: on.clone() };
            }
            Structure::ItemReady { item } => {
                self.item_mut(seq, *item)?.state = ItemState::Ready;
            }
            Structure::ItemDone { item, evidence } => {
                self.item_mut(seq, *item)?.state = ItemState::Done {
                    evidence: evidence.clone(),
                };
            }
            Structure::ItemMerged { item, commit } => {
                let i = self.item_mut(seq, *item)?;
                let evidence = match &i.state {
                    ItemState::Done { evidence } => evidence.clone(),
                    _ => Vec::new(),
                };
                i.state = ItemState::Merged {
                    evidence,
                    commit: commit.clone(),
                };
            }
            Structure::ActionRaised {
                action,
                by,
                blocks,
                informs,
            } => {
                self.actions.insert(
                    *action,
                    crate::primitives::Action {
                        id: *action,
                        by: by.clone(),
                        at: seq,
                        blocks: blocks.clone(),
                        informs: informs.clone(),
                        check: None,
                        state: crate::primitives::ActionState::Open,
                    },
                );
                for i in blocks.iter().chain(informs) {
                    self.index
                        .actions_about
                        .entry(*i)
                        .or_default()
                        .push(*action);
                }
            }
            Structure::ActionCheckSet { action } => {
                self.actions
                    .get_mut(action)
                    .ok_or(FoldError::UnknownAction {
                        seq,
                        action: *action,
                    })?
                    .check = Some(seq);
            }
            Structure::ActionDone { action, evidence } => {
                let a = self
                    .actions
                    .get_mut(action)
                    .ok_or(FoldError::UnknownAction {
                        seq,
                        action: *action,
                    })?;
                a.state = crate::primitives::ActionState::Done {
                    at: seq,
                    evidence: evidence.clone(),
                };
                // Clear it from any item blocked on it; the move back to
                // Ready is its own event, emitted by apply.
                for i in self.items.values_mut() {
                    if let ItemState::Blocked { on } = &mut i.state {
                        on.retain(|b| *b != Blocker::Action(*action));
                    }
                }
            }
            Structure::RunStarted { seat } => {
                self.running.insert(seat.clone(), seq);
                // A new run is the person's retry after fixing the
                // setup: what the last run reported is cleared.
                self.faults.remove(seat);
            }
            Structure::SetupFault { seat } => {
                self.faults.entry(seat.clone()).or_default().push(seq);
            }
            Structure::RunStopped { seat, .. } => {
                self.running.remove(seat);
            }
            Structure::SeatPaused { seat } => {
                self.paused.insert(seat.clone());
            }
            Structure::SeatResumed { seat } => {
                self.paused.remove(seat);
            }
            Structure::ItemSuperseded { item, by } => {
                self.item_mut(seq, *item)?.state = ItemState::Superseded { by: *by };
                // In id order, as a scan of the items gives it.
                let r = self.index.replaced.entry(*by).or_default();
                r.push(*item);
                r.sort();
            }
            Structure::DecisionRaised {
                decision,
                class,
                by,
                about,
                options,
                recommended,
            } => {
                self.decisions.insert(
                    *decision,
                    Decision {
                        id: *decision,
                        class: *class,
                        by: by.clone(),
                        at: seq,
                        options: *options,
                        recommended: *recommended,
                        about: *about,
                        state: DecisionState::Open,
                        explained: None,
                    },
                );
                if let Some(a) = about {
                    self.index
                        .decisions_about
                        .entry(*a)
                        .or_default()
                        .push(*decision);
                }
            }
            Structure::DecisionExplained {
                decision,
                by,
                pros,
                cons,
            } => {
                let d = self
                    .decisions
                    .get_mut(decision)
                    .ok_or(FoldError::UnknownDecision {
                        seq,
                        decision: *decision,
                    })?;
                d.explained = Some(crate::primitives::Explanation {
                    at: seq,
                    by: by.clone(),
                    pros: pros.clone(),
                    cons: cons.clone(),
                });
            }
            Structure::DecisionTaken {
                decision,
                by,
                chosen,
            } => {
                let d = self
                    .decisions
                    .get_mut(decision)
                    .ok_or(FoldError::UnknownDecision {
                        seq,
                        decision: *decision,
                    })?;
                d.state = DecisionState::Taken {
                    by: by.clone(),
                    chosen: *chosen,
                    at: seq,
                };
                // Clear this decision from any item blocked on it. The
                // move back to Ready is its own event, emitted by apply.
                for i in self.items.values_mut() {
                    if let ItemState::Blocked { on } = &mut i.state {
                        on.retain(|b| *b != Blocker::Decision(*decision));
                    }
                }
            }
            Structure::CheckRan {
                item,
                check,
                passed,
                evidence,
            } => {
                self.item_mut(seq, *item)?.check_runs.push(CheckRun {
                    check: check.clone(),
                    passed: *passed,
                    evidence: *evidence,
                    at: seq,
                });
            }
            Structure::MessageSent {
                message,
                from,
                to,
                kind,
                about,
            } => {
                self.messages.push(Message {
                    id: *message,
                    from: from.clone(),
                    to: to.clone(),
                    kind: *kind,
                    about: *about,
                    at: seq,
                });
            }
        }
        self.len = seq;
        self.head = e.hash;
        Ok(())
    }

    fn item_mut(&mut self, seq: u64, item: ItemId) -> Result<&mut Item, FoldError> {
        self.items
            .get_mut(&item)
            .ok_or(FoldError::UnknownItem { seq, item })
    }
}
