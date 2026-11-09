//! `ris import <file>`: items and standing decisions from a TOML file.
//!
//! ```toml
//! [[item]]
//! key = "auth"                  # optional; names it for `depends`
//! goal = "Add login"
//! checks = ["test"]
//! depends = ["I-3"]             # earlier keys in this file, or existing ids
//!
//! [[decision]]
//! question = "Which database?"
//! options = ["sqlite", "postgres"]
//! answer = 0                    # or: other = "an answer outside the options"
//! rationale = "Why it was decided"
//! ```
//!
//! The whole file is validated first: every text field scanned under
//! the policy, every check declared, every dependency resolved,
//! every answer in range. Only a file with no problems is written, so a
//! bad entry anywhere means nothing is imported. Decisions are standing
//! decisions, recorded as the person's; importing them needs the
//! person's seat.

use serde::Deserialize;

use ris_core::command::{Choice, Command};
use ris_core::ids::{Actor, CheckId, ItemId};
use ris_core::primitives::DecisionClass;
use ris_core::state::State;
use ris_core::text::{Policy, Text};

use crate::config::ChecksConfig;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    item: Vec<RawItem>,
    #[serde(default)]
    decision: Vec<RawDecision>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    key: Option<String>,
    goal: String,
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    depends: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    question: String,
    options: Vec<String>,
    answer: Option<usize>,
    other: Option<String>,
    rationale: String,
}

/// A validated import: the commands to run, in order.
pub struct Plan {
    pub commands: Vec<Command>,
    pub items: usize,
    pub decisions: usize,
}

/// Parse and validate `source` against the current state, the declared
/// checks and the policy. Every problem is reported, not just the first;
/// a refusal never echoes the text that caused it.
pub fn plan(
    source: &str,
    state: &State,
    checks: &ChecksConfig,
    policy: &Policy,
) -> Result<Plan, Vec<String>> {
    let file: File = toml::from_str(source).map_err(|e| vec![format!("the file: {e}")])?;
    let mut problems = Vec::new();
    let mut commands = Vec::new();
    let text = |label: String, field: &str, raw: &str, problems: &mut Vec<String>| match Text::new(
        field, raw, policy,
    ) {
        Ok(t) => Some(t),
        Err(r) => {
            problems.push(format!("{label}: {r}"));
            None
        }
    };

    let first = state.next_item_id().0;
    let mut keys: Vec<(String, ItemId)> = Vec::new();
    for (n, it) in file.item.iter().enumerate() {
        let label = format!("item {}", n + 1);
        let id = ItemId(first + n as u32);
        let goal = text(label.clone(), "goal", &it.goal, &mut problems);
        for c in &it.checks {
            if !checks.checks.contains_key(c) {
                problems.push(format!(
                    "{label}: check `{c}` is not in .osiris/checks.toml"
                ));
            }
        }
        let mut depends = Vec::new();
        for d in &it.depends {
            if let Some((_, dep)) = keys.iter().find(|(k, _)| k == d) {
                depends.push(*dep);
            } else if let Ok(dep) = d.parse::<ItemId>()
                && state.items.contains_key(&dep)
            {
                depends.push(dep);
            } else {
                problems.push(format!(
                    "{label}: depends on `{d}`, which is neither an earlier key in this file nor an existing item"
                ));
            }
        }
        if let Some(k) = &it.key {
            if keys.iter().any(|(existing, _)| existing == k) {
                problems.push(format!("{label}: key `{k}` is used twice"));
            }
            keys.push((k.clone(), id));
        }
        if let Some(goal) = goal {
            commands.push(Command::ItemAdd {
                goal,
                checks: it.checks.iter().cloned().map(CheckId).collect(),
                depends_on: depends,
            });
        }
    }

    let first_decision = state.next_decision_id().0;
    for (n, d) in file.decision.iter().enumerate() {
        let label = format!("decision {}", n + 1);
        let question = text(label.clone(), "question", &d.question, &mut problems);
        let options: Vec<Option<Text>> = d
            .options
            .iter()
            .enumerate()
            .map(|(i, o)| text(label.clone(), &format!("option.{i}"), o, &mut problems))
            .collect();
        let rationale = text(label.clone(), "rationale", &d.rationale, &mut problems);
        if d.options.len() < 2 {
            problems.push(format!("{label}: give at least two options"));
        }
        let choice = match (d.answer, &d.other) {
            (Some(i), None) if i < d.options.len() => Some(Choice::Option(i)),
            (Some(i), None) => {
                problems.push(format!(
                    "{label}: answer {i} is not one of its {} options",
                    d.options.len()
                ));
                None
            }
            (None, Some(o)) => text(label.clone(), "other", o, &mut problems).map(Choice::Other),
            _ => {
                problems.push(format!("{label}: give exactly one of `answer` or `other`"));
                None
            }
        };
        let options: Option<Vec<Text>> = options.into_iter().collect();
        if let (Some(question), Some(options), Some(rationale), Some(chosen)) =
            (question, options, rationale, choice)
        {
            let recommended = match &chosen {
                Choice::Option(i) => *i,
                Choice::Other(_) => 0,
            };
            commands.push(Command::DecisionRaise {
                by: Actor::Person,
                about: None,
                class: DecisionClass::Substantive,
                question,
                options,
                recommended,
                rationale: rationale.clone(),
            });
            commands.push(Command::Decide {
                decision: ris_core::ids::DecisionId(first_decision + n as u32),
                by: Actor::Person,
                chosen,
                rationale,
            });
        }
    }

    if problems.is_empty() {
        Ok(Plan {
            commands,
            items: file.item.len(),
            decisions: file.decision.len(),
        })
    } else {
        Err(problems)
    }
}
