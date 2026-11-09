//! The transition table: for each primitive, which command may leave
//! each of its states, and where it goes.
//!
//! This is data, and it is the specification. The core's `apply` is
//! tested against it: every (state, command) pair here has exactly one
//! outcome, and `apply` must produce it. A pair not in the table is a
//! refusal. Adding a state variant without extending the table fails
//! the completeness test in this module.

use serde::{Deserialize, Serialize};

/// The commands of the first slice, by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommandKind {
    PhaseOpen,
    PhaseClose,
    ItemAdd,
    Claim,
    /// Raising a mechanical decision: logged, the item carries on.
    RaiseMechanical,
    /// Raising a substantive decision: queued, the item is set aside.
    RaiseSubstantive,
    Decide,
    /// Recording a check's run against an item.
    CheckRan,
    Done,
    Merge,
    MessageSend,
    /// Returning a claimed item to ready: by its holder, or the person.
    Release,
    /// An auditor recording that it has reviewed an item.
    Audit,
}

impl CommandKind {
    /// Every kind, for completeness tests.
    pub const ALL: &[CommandKind] = &[
        Self::PhaseOpen,
        Self::PhaseClose,
        Self::ItemAdd,
        Self::Claim,
        Self::RaiseMechanical,
        Self::RaiseSubstantive,
        Self::Decide,
        Self::CheckRan,
        Self::Done,
        Self::Merge,
        Self::MessageSend,
        Self::Release,
        Self::Audit,
    ];
}

/// The shape of an item's state, without its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemStateKind {
    Ready,
    Claimed,
    Blocked,
    Done,
    Merged,
}

impl ItemStateKind {
    pub const ALL: &[ItemStateKind] = &[
        Self::Ready,
        Self::Claimed,
        Self::Blocked,
        Self::Done,
        Self::Merged,
    ];
}

/// The shape of a phase's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhaseStateKind {
    Open,
    Closed,
}

impl PhaseStateKind {
    pub const ALL: &[PhaseStateKind] = &[Self::Open, Self::Closed];
}

/// The shape of a decision's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecisionStateKind {
    Open,
    Taken,
    Superseded,
}

impl DecisionStateKind {
    pub const ALL: &[DecisionStateKind] = &[Self::Open, Self::Taken, Self::Superseded];
}

/// What a command does to a state. Commands not listed for a state are
/// refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move<K> {
    /// The state changes to this kind.
    To(K),
    /// The command applies and is logged; the state does not change.
    Stay,
}

/// Commands that act on an item, from each item state.
pub const ITEM: &[(ItemStateKind, CommandKind, Move<ItemStateKind>)] = {
    use CommandKind as C;
    use ItemStateKind as S;
    &[
        (S::Ready, C::Claim, Move::To(S::Claimed)),
        (S::Claimed, C::RaiseMechanical, Move::Stay),
        (S::Claimed, C::RaiseSubstantive, Move::To(S::Blocked)),
        (S::Claimed, C::CheckRan, Move::Stay),
        (S::Claimed, C::Done, Move::To(S::Done)),
        // Until leases arrive with the daemon, a stuck claim is released
        // by hand.
        (S::Claimed, C::Release, Move::To(S::Ready)),
        // The last blocker clearing returns the item to Ready; while
        // blockers remain, the decision is logged and the item stays.
        (S::Blocked, C::Decide, Move::To(S::Ready)),
        // Checks may run again on a done item (the merge runs them
        // all); its state does not change.
        (S::Done, C::CheckRan, Move::Stay),
        (S::Done, C::Merge, Move::To(S::Merged)),
        // Audit never holds anything up: it is recorded against a done
        // or merged item, and findings are fixed forward.
        (S::Done, C::Audit, Move::Stay),
        (S::Merged, C::Audit, Move::Stay),
        // Merged: nothing. Changed only by a further item.
    ]
};

/// Commands that act on a phase, from each phase state. `PhaseOpen` is
/// refused while a phase is open: one at a time.
pub const PHASE: &[(PhaseStateKind, CommandKind, Move<PhaseStateKind>)] = {
    use CommandKind as C;
    use PhaseStateKind as S;
    &[
        (S::Open, C::ItemAdd, Move::Stay),
        (S::Open, C::PhaseClose, Move::To(S::Closed)),
        (S::Closed, C::PhaseOpen, Move::To(S::Open)),
    ]
};

/// Commands that act on a decision, from each decision state.
pub const DECISION: &[(DecisionStateKind, CommandKind, Move<DecisionStateKind>)] = {
    use CommandKind as C;
    use DecisionStateKind as S;
    &[
        (S::Open, C::Decide, Move::To(S::Taken)),
        // Taken and Superseded: nothing. A new decision supersedes.
    ]
};

/// Look up a move. `None` is a refusal.
pub fn lookup<K: Copy + PartialEq>(
    table: &[(K, CommandKind, Move<K>)],
    state: K,
    command: CommandKind,
) -> Option<Move<K>> {
    table
        .iter()
        .find(|(s, c, _)| *s == state && *c == command)
        .map(|(_, _, m)| *m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// `terminal` lists the states no command leaves or reaches: they
    /// are written by another mechanism (a later decision superseding
    /// this one) and must be named here deliberately.
    fn assert_well_formed<K: Copy + PartialEq + Eq + std::hash::Hash + std::fmt::Debug>(
        table: &[(K, CommandKind, Move<K>)],
        all_states: &[K],
        terminal: &[K],
    ) {
        let mut seen = HashSet::new();
        for (state, command, _) in table {
            assert!(
                seen.insert((*state, *command)),
                "duplicate entry for {state:?} + {command:?}"
            );
            assert!(all_states.contains(state), "{state:?} is not a known state");
        }
        // Every state kind is mentioned somewhere as a source or a
        // target, or is explicitly terminal below.
        let mentioned: HashSet<K> = table
            .iter()
            .flat_map(|(s, _, m)| match m {
                Move::To(t) => vec![*s, *t],
                Move::Stay => vec![*s],
            })
            .collect();
        for state in all_states {
            assert!(
                mentioned.contains(state) || terminal.contains(state),
                "{state:?} appears nowhere in the table; add it or mark it terminal"
            );
        }
    }

    #[test]
    fn item_table_is_well_formed() {
        assert_well_formed(ITEM, ItemStateKind::ALL, &[]);
    }

    #[test]
    fn phase_table_is_well_formed() {
        assert_well_formed(PHASE, PhaseStateKind::ALL, &[]);
    }

    #[test]
    fn decision_table_is_well_formed() {
        // Superseded is a target of no command: it is written by a
        // later decision's Taken, not by a command on this one.
        assert_well_formed(
            DECISION,
            DecisionStateKind::ALL,
            &[DecisionStateKind::Superseded],
        );
    }

    #[test]
    fn merged_and_taken_are_terminal() {
        // Nothing moves an item out of Merged: the only command it
        // accepts is a review, which leaves it where it is.
        for command in CommandKind::ALL {
            let m = lookup(ITEM, ItemStateKind::Merged, *command);
            assert!(
                m.is_none() || m == Some(Move::Stay),
                "{command:?} moves a merged item"
            );
            assert_eq!(lookup(DECISION, DecisionStateKind::Taken, *command), None);
        }
        assert_eq!(
            lookup(ITEM, ItemStateKind::Merged, CommandKind::Audit),
            Some(Move::Stay)
        );
    }

    #[test]
    fn a_second_claim_is_refused() {
        assert_eq!(
            lookup(ITEM, ItemStateKind::Claimed, CommandKind::Claim),
            None
        );
        assert_eq!(
            lookup(ITEM, ItemStateKind::Blocked, CommandKind::Claim),
            None
        );
        assert_eq!(lookup(ITEM, ItemStateKind::Done, CommandKind::Claim), None);
    }

    #[test]
    fn only_one_phase_is_open() {
        assert_eq!(
            lookup(PHASE, PhaseStateKind::Open, CommandKind::PhaseOpen),
            None
        );
    }
}
