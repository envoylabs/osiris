//! The brief printed by `ris claim`: what an agent needs to start an
//! item, within a fixed size.
//!
//! Three sections, in an order fixed so that what changes least comes
//! first and a provider's prompt cache can reuse the prefix:
//!
//! 1. **rules:** where the agents' rules are and the hash of their
//!    content. They are named, not inlined: the agent tool already
//!    loads them through its rules file, so inlining would put the same
//!    text in context twice.
//! 2. **project:** the open phase and its goal, and the seat's role.
//! 3. **item:** its goal, checks, dependencies, and the decisions that
//!    touch it (raised about it or about an item it depends on, or
//!    about an item either of those replaced).
//!
//! The text form is capped at [`BRIEF_MAX`] bytes. Decisions are cut
//! first, oldest first, with a line saying how many were left out and
//! where to read them; the rules, project and the item's own goal are
//! never cut.

use std::collections::BTreeSet;

use ris_core::ids::ItemId;
use ris_core::primitives::{Decision, DecisionState, ItemState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::project::Loaded;

/// The most a brief's text form may take, in bytes.
pub const BRIEF_MAX: usize = 4096;

/// The rules as the brief names them: a path relative to the project
/// root, or `None` when the project has no rules file and the built-in
/// rules (`ris rules`) are meant; and their content.
pub struct Rules<'a> {
    pub path: Option<&'a str>,
    pub content: &'a str,
}

/// The brief for `item`, claimed by a seat with `role`, as text and as
/// JSON.
pub fn brief(l: &Loaded, item: ItemId, role: &str, rules: &Rules) -> (String, Value) {
    let i = &l.state.items[&item];
    let rules_hash: String = Sha256::digest(rules.content.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    // The open phase: an item carried over from an earlier one is
    // worked under the current goal.
    let phase = l.state.open_phase().unwrap_or(&l.state.phases[&i.phase]);
    let phase_goal = l.text(phase.at, "goal");
    let goal = l.text(i.at, "goal");
    let checks: Vec<String> = i.checks.iter().map(ToString::to_string).collect();

    let mut head = String::new();
    head.push_str(&match rules.path {
        Some(path) => format!("rules:   {path} ({rules_hash}), loaded by your rules file\n"),
        None => format!("rules:   `ris rules` ({rules_hash}); no rules file in this project\n"),
    });
    head.push_str(&format!(
        "project: {} {phase_goal}\n         you are {role}\n",
        phase.id
    ));
    head.push_str(&format!("item:    {item} {goal}\n"));
    head.push_str(&format!(
        "  checks: {}\n",
        if checks.is_empty() {
            "none".to_owned()
        } else {
            checks.join(", ")
        }
    ));
    let mut deps_json = Vec::new();
    for d in &i.depends_on {
        let dep = &l.state.items[d];
        let state = state_word(&dep.state);
        head.push_str(&format!(
            "  depends on {d} ({state}): {}\n",
            l.text(dep.at, "goal")
        ));
        deps_json.push(json!({ "item": d.to_string(), "state": state }));
    }

    // Decisions raised about this item or what it depends on, newest
    // first, so a cut drops the oldest.
    // An item stands for every item it replaced, so a decision about a
    // superseded item touches its replacement: each item counts with its
    // lineage, read from the reverse index.
    let touching: BTreeSet<ItemId> = std::iter::once(item)
        .chain(i.depends_on.iter().copied())
        .map(|t| l.state.current(t))
        .flat_map(|t| l.state.lineage(t))
        .collect();
    let ids: BTreeSet<_> = touching
        .iter()
        .flat_map(|t| l.state.decisions_about(*t))
        .copied()
        .collect();
    let mut decisions: Vec<&Decision> = ids.iter().map(|d| &l.state.decisions[d]).collect();
    decisions.reverse();

    let mut lines = Vec::new();
    let mut decisions_json = Vec::new();
    for d in &decisions {
        let (state, answer) = match &d.state {
            DecisionState::Open => ("open", "waiting for the person".to_owned()),
            DecisionState::Taken { chosen, at, .. } if *chosen >= d.options => (
                "taken",
                format!("{} ({})", l.text(*at, "other"), l.text(*at, "rationale")),
            ),
            DecisionState::Taken { chosen, at, .. } => (
                "taken",
                format!(
                    "{} ({})",
                    l.text(d.at, &format!("option.{chosen}")),
                    l.text(*at, "rationale")
                ),
            ),
            DecisionState::Superseded(by) => ("superseded", format!("see {by}")),
        };
        lines.push(format!(
            "  {} {} → {answer}\n",
            d.id,
            l.text(d.at, "question")
        ));
        decisions_json.push(json!({ "id": d.id.to_string(), "state": state }));
    }

    let mut text = head;
    let mut kept = 0;
    let more = |n: usize| format!("  and {n} older decision(s): ris show {item}\n");
    if !lines.is_empty() {
        text.push_str("  decisions:\n");
        for (n, line) in lines.iter().enumerate() {
            let left = lines.len() - n - 1;
            let tail = if left > 0 { more(left).len() } else { 0 };
            if text.len() + line.len() + tail > BRIEF_MAX {
                break;
            }
            text.push_str(line);
            kept += 1;
        }
        if kept < lines.len() {
            text.push_str(&more(lines.len() - kept));
        }
    }
    // The fixed parts alone can exceed the cap only through a very long
    // goal; cut at a character boundary rather than refuse the claim.
    if text.len() > BRIEF_MAX {
        let mut cut = BRIEF_MAX - 4;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("...\n");
    }

    let json = json!({
        "rules": { "path": rules.path, "hash": rules_hash },
        "project": { "phase": phase.id.to_string(), "goal": phase_goal, "role": role },
        "item": item.to_string(),
        "goal": goal,
        "checks": checks,
        "depends_on": deps_json,
        "decisions": decisions_json[..kept].to_vec(),
        "decisions_left_out": lines.len() - kept,
    });
    (text.trim_end().to_owned(), json)
}

fn state_word(s: &ItemState) -> &'static str {
    match s {
        ItemState::Ready => "ready",
        ItemState::Claimed { .. } => "claimed",
        ItemState::Blocked { .. } => "blocked",
        ItemState::Done { .. } => "done",
        ItemState::Merged { .. } => "merged",
        ItemState::Superseded { .. } => "superseded",
    }
}
