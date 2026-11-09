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
}

/// A structural reference that the log should have made impossible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldError {
    UnknownItem { seq: u64, item: ItemId },
    UnknownDecision { seq: u64, decision: DecisionId },
    UnknownPhase { seq: u64, phase: PhaseId },
}

impl std::fmt::Display for FoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownItem { seq, item } => write!(f, "event {seq} refers to unknown {item}"),
            Self::UnknownDecision { seq, decision } => {
                write!(f, "event {seq} refers to unknown {decision}")
            }
            Self::UnknownPhase { seq, phase } => write!(f, "event {seq} refers to unknown {phase}"),
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
        Ok(state)
    }

    /// The phase that is open, if one is.
    pub fn open_phase(&self) -> Option<&Phase> {
        self.phases
            .values()
            .find(|p| matches!(p.state, PhaseState::Open))
    }

    /// Items an agent may claim now: ready, with every dependency landed.
    pub fn ready(&self) -> impl Iterator<Item = &Item> {
        self.items.values().filter(|i| {
            matches!(i.state, ItemState::Ready) && self.unlanded_dependencies(i).is_empty()
        })
    }

    /// Dependencies of `item` that have not landed.
    pub fn unlanded_dependencies(&self, item: &Item) -> Vec<ItemId> {
        item.depends_on
            .iter()
            .filter(|d| {
                !matches!(
                    self.items.get(d).map(|i| &i.state),
                    Some(ItemState::Landed { .. })
                )
            })
            .copied()
            .collect()
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
                        at: seq,
                        checks: checks.clone(),
                        depends_on: depends_on.clone(),
                        check_runs: Vec::new(),
                        state: ItemState::Ready,
                    },
                );
            }
            Structure::ItemClaimed { item, agent } => {
                self.item_mut(seq, *item)?.state = ItemState::Claimed {
                    agent: agent.clone(),
                };
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
            Structure::ItemLanded { item, commit } => {
                let i = self.item_mut(seq, *item)?;
                let evidence = match &i.state {
                    ItemState::Done { evidence } => evidence.clone(),
                    _ => Vec::new(),
                };
                i.state = ItemState::Landed {
                    evidence,
                    commit: commit.clone(),
                };
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
                    },
                );
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
