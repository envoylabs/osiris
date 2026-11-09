//! Refusals: the typed answer when a command cannot be applied.
//!
//! A refusal is part of the contract, not an error path: an agent reads
//! it and knows what to do next. Variants are added as commands are.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{AgentName, DecisionId, ItemId};
use crate::text::Category;
use crate::transitions::CommandKind;

/// Why a command was not applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// A free-text field failed the confidentiality scan. The match is
    /// never echoed: only the field and the category are named.
    Confidential {
        /// The field that was refused, as the caller named it.
        field: String,
        /// What kind of thing the scan found.
        category: Category,
    },
    /// The command needs an open phase and there is none.
    NoOpenPhase,
    /// Only one phase is open at a time.
    PhaseAlreadyOpen,
    /// No item has this id.
    UnknownItem(ItemId),
    /// No decision has this id.
    UnknownDecision(DecisionId),
    /// The command is not allowed from the item's current state. The
    /// transition table is the specification.
    IllegalItemTransition {
        item: ItemId,
        /// The item's state, as the table names it.
        state: &'static str,
        command: CommandKind,
    },
    /// The command is not allowed from the decision's current state.
    IllegalDecisionTransition {
        decision: DecisionId,
        state: &'static str,
        command: CommandKind,
    },
    /// The item depends on items that have not landed.
    DependenciesOpen {
        item: ItemId,
        waiting_on: Vec<ItemId>,
    },
    /// Only the agent holding an item may act on it.
    NotTheClaimant { item: ItemId, holder: AgentName },
    /// Done means evidence; none was attached.
    NoEvidence(ItemId),
    /// The chosen option is out of range.
    NoSuchOption {
        decision: DecisionId,
        chosen: usize,
        options: usize,
    },
    /// A decision needs at least two options, and the recommendation
    /// must be one of them.
    MalformedDecision,
    /// A substantive decision is the person's to take.
    SubstantiveIsForThePerson(DecisionId),
    /// A message must go to a named agent other than its sender.
    MessageToSelf,
}

impl Refusal {
    /// What the actor should do about it, in one line.
    pub fn next_step(&self) -> String {
        match self {
            Self::Confidential { field, category } => format!(
                "remove the {} from `{field}` and try again; \
                 harness state and commits never carry one",
                category.noun()
            ),
            Self::NoOpenPhase => "open a phase first: `ris phase open`".into(),
            Self::PhaseAlreadyOpen => {
                "close the open phase first: `ris phase close`, after review".into()
            }
            Self::UnknownItem(_) => "list items with `ris ready` or `ris report`".into(),
            Self::UnknownDecision(_) => "list decisions with `ris inbox`".into(),
            Self::IllegalItemTransition { state, command, .. } => {
                format!("an item in state {state} does not accept {command:?}; see `ris report`")
            }
            Self::IllegalDecisionTransition { state, .. } => {
                format!("a decision in state {state} is settled; raise a new one to supersede it")
            }
            Self::DependenciesOpen { waiting_on, .. } => format!(
                "wait for {} to land, or take another item from `ris ready`",
                join(waiting_on)
            ),
            Self::NotTheClaimant { holder, .. } => {
                format!("the item is held by {holder}; message them or take another item")
            }
            Self::NoEvidence(_) => {
                "run the item's checks, or attach evidence with `--evidence <file>`".into()
            }
            Self::NoSuchOption { options, .. } => {
                format!("choose an option from 0 to {}", options.saturating_sub(1))
            }
            Self::MalformedDecision => "give at least two options and recommend one of them".into(),
            Self::SubstantiveIsForThePerson(_) => {
                "leave it in the queue; the person takes it at the next check-in".into()
            }
            Self::MessageToSelf => "address the message to another agent".into(),
        }
    }
}

fn join(ids: &[ItemId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confidential { field, category } => {
                write!(f, "refused: `{field}` contains {}", category.description())
            }
            Self::NoOpenPhase => f.write_str("refused: no phase is open"),
            Self::PhaseAlreadyOpen => f.write_str("refused: a phase is already open"),
            Self::UnknownItem(id) => write!(f, "refused: no item {id}"),
            Self::UnknownDecision(id) => write!(f, "refused: no decision {id}"),
            Self::IllegalItemTransition {
                item,
                state,
                command,
            } => write!(
                f,
                "refused: {item} is {state}, which does not accept {command:?}"
            ),
            Self::IllegalDecisionTransition {
                decision,
                state,
                command,
            } => write!(
                f,
                "refused: {decision} is {state}, which does not accept {command:?}"
            ),
            Self::DependenciesOpen { item, waiting_on } => {
                write!(f, "refused: {item} waits on {}", join(waiting_on))
            }
            Self::NotTheClaimant { item, holder } => {
                write!(f, "refused: {item} is held by {holder}")
            }
            Self::NoEvidence(item) => write!(f, "refused: {item} has no evidence attached"),
            Self::NoSuchOption {
                decision,
                chosen,
                options,
            } => write!(
                f,
                "refused: {decision} has {options} options; {chosen} is not one of them"
            ),
            Self::MalformedDecision => {
                f.write_str("refused: a decision needs options and a recommendation among them")
            }
            Self::SubstantiveIsForThePerson(id) => {
                write!(
                    f,
                    "refused: {id} is substantive and is the person's to take"
                )
            }
            Self::MessageToSelf => {
                f.write_str("refused: a message cannot be addressed to its sender")
            }
        }
    }
}

impl std::error::Error for Refusal {}
