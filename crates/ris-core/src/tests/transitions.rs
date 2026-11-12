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
fn action_table_is_well_formed() {
    assert_well_formed(ACTION, ActionStateKind::ALL, &[]);
}

#[test]
fn superseded_is_final() {
    for command in CommandKind::ALL {
        assert_eq!(lookup(ITEM, ItemStateKind::Superseded, *command), None);
    }
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
