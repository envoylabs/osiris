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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Attaching the full case for an open decision: pros and cons per
    /// option and why the recommendation holds.
    Explain,
    /// The person replacing an item that has not been started with
    /// another.
    Supersede,
    /// The person bringing an item that has not been started from the
    /// backlog into the open phase.
    BringIn,
    /// The person pausing seats, or resuming them.
    Pause,
    Resume,
    /// The runner recording that a seat's agent started or stopped.
    RunStart,
    RunStop,
    /// A seat reporting a fault in its setup.
    SetupFault,
    /// Raising a person action; it sets aside the items it blocks.
    RaiseAction,
    /// The person recording an action done; items blocked only on it
    /// return to ready.
    ActionDone,
    /// The person giving an open action a check that closes it.
    SetActionCheck,
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
        Self::Explain,
        Self::Supersede,
        Self::BringIn,
        Self::Pause,
        Self::Resume,
        Self::RaiseAction,
        Self::ActionDone,
        Self::SetActionCheck,
        Self::RunStart,
        Self::RunStop,
        Self::SetupFault,
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
    Superseded,
}

impl ItemStateKind {
    pub const ALL: &[ItemStateKind] = &[
        Self::Ready,
        Self::Claimed,
        Self::Blocked,
        Self::Done,
        Self::Merged,
        Self::Superseded,
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
        // An item nobody has started may be replaced; one in progress
        // is released first, and one done or merged stands.
        (S::Ready, C::Supersede, Move::To(S::Superseded)),
        (S::Blocked, C::Supersede, Move::To(S::Superseded)),
        // Likewise an item nobody has started may be brought into the
        // open phase; its state does not change, only its phase.
        (S::Ready, C::BringIn, Move::Stay),
        (S::Blocked, C::BringIn, Move::Stay),
        // A person action that blocks an item sets it aside, claimed or
        // not, as a substantive decision does; one more blocker on a
        // blocked item leaves it blocked. Done, it returns the item to
        // Ready when nothing else blocks it.
        (S::Ready, C::RaiseAction, Move::To(S::Blocked)),
        (S::Claimed, C::RaiseAction, Move::To(S::Blocked)),
        (S::Blocked, C::RaiseAction, Move::Stay),
        (S::Blocked, C::ActionDone, Move::To(S::Ready)),
        // Superseded: nothing. Final.
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
        // An open decision can be explained, and explained again: the
        // latest explanation is the one shown.
        (S::Open, C::Explain, Move::Stay),
        // Taken and Superseded: nothing. A new decision supersedes.
    ]
};

/// The shape of a person action's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionStateKind {
    Open,
    Done,
}

impl ActionStateKind {
    pub const ALL: &[ActionStateKind] = &[Self::Open, Self::Done];
}

/// Commands that act on a person action, from each state.
pub const ACTION: &[(ActionStateKind, CommandKind, Move<ActionStateKind>)] = {
    use ActionStateKind as S;
    use CommandKind as C;
    // Done: nothing. Final.
    &[
        (S::Open, C::ActionDone, Move::To(S::Done)),
        (S::Open, C::SetActionCheck, Move::Stay),
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
#[path = "tests/transitions.rs"]
mod tests;
