//! Rendering: what each command prints, as text and as JSON.
//!
//! Agents read this output, so the text is short and stable, and the
//! JSON carries the same facts with their ids.

use ris_core::ids::{Actor, AgentName};
use ris_core::primitives::{
    Decision, DecisionClass, DecisionState, Item, ItemState, Message, PhaseState,
};
use serde_json::{Value, json};

use crate::project::Loaded;

/// One command's output.
pub struct Output {
    pub text: String,
    pub json: Value,
}

impl Output {
    pub fn new(text: impl Into<String>, json: Value) -> Self {
        Self {
            text: text.into(),
            json,
        }
    }
}

fn state_name(s: &ItemState) -> &'static str {
    match s {
        ItemState::Ready => "ready",
        ItemState::Claimed { .. } => "claimed",
        ItemState::Blocked { .. } => "blocked",
        ItemState::Done { .. } => "done",
        ItemState::Merged { .. } => "merged",
    }
}

fn item_json(l: &Loaded, i: &Item) -> Value {
    let mut v = json!({
        "id": i.id.to_string(),
        "phase": i.phase.to_string(),
        "goal": l.text(i.at, "goal"),
        "state": state_name(&i.state),
        "checks": i.checks.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "depends_on": i.depends_on.iter().map(ToString::to_string).collect::<Vec<_>>(),
    });
    match &i.state {
        ItemState::Claimed { agent } => v["agent"] = json!(agent.to_string()),
        ItemState::Blocked { on } => {
            v["blocked_on"] = json!(on.iter().map(|b| format!("{b:?}")).collect::<Vec<_>>())
        }
        ItemState::Done { evidence } => {
            v["evidence"] = json!(evidence.iter().map(ToString::to_string).collect::<Vec<_>>())
        }
        ItemState::Merged { evidence, commit } => {
            v["evidence"] = json!(evidence.iter().map(ToString::to_string).collect::<Vec<_>>());
            v["commit"] = json!(commit.to_string());
        }
        ItemState::Ready => {}
    }
    v
}

fn item_line(l: &Loaded, i: &Item) -> String {
    let who = match &i.state {
        ItemState::Claimed { agent } => format!(" ({agent})"),
        ItemState::Merged { commit, .. } => format!(" ({})", short(&commit.0)),
        _ => String::new(),
    };
    format!(
        "{:<5} {:<8} {}{}",
        i.id.to_string(),
        state_name(&i.state),
        l.text(i.at, "goal"),
        who
    )
}

fn short(s: &str) -> &str {
    &s[..s.len().min(8)]
}

fn decision_json(l: &Loaded, d: &Decision) -> Value {
    let options: Vec<&str> = (0..d.options)
        .map(|n| l.text(d.at, &format!("option.{n}")))
        .collect();
    let mut v = json!({
        "id": d.id.to_string(),
        "class": match d.class { DecisionClass::Mechanical => "mechanical", DecisionClass::Substantive => "substantive" },
        "by": d.by.to_string(),
        "about": d.about.map(|i| i.to_string()),
        "question": l.text(d.at, "question"),
        "options": options,
        "recommended": d.recommended,
        "state": match d.state { DecisionState::Open => "open", DecisionState::Taken { .. } => "taken", DecisionState::Superseded(_) => "superseded" },
    });
    if let DecisionState::Taken { by, chosen, at } = &d.state {
        v["taken_by"] = json!(by.to_string());
        v["chosen"] = json!(chosen);
        v["rationale"] = json!(l.text(*at, "rationale"));
    }
    v
}

/// A decision in full: question, options, recommendation, outcome.
fn decision_block(l: &Loaded, d: &Decision) -> String {
    let mut s = format!(
        "{} [{}] {}{}\n",
        d.id,
        match d.class {
            DecisionClass::Mechanical => "mechanical",
            DecisionClass::Substantive => "substantive",
        },
        l.text(d.at, "question"),
        d.about
            .map(|i| format!("  (about {i})"))
            .unwrap_or_default()
    );
    for n in 0..d.options {
        let mark = if n == d.recommended { "*" } else { " " };
        s.push_str(&format!(
            "   {mark} {n}: {}\n",
            l.text(d.at, &format!("option.{n}"))
        ));
    }
    match &d.state {
        DecisionState::Open => s.push_str("     open; * marks the recommendation\n"),
        DecisionState::Taken { by, chosen, at } => s.push_str(&format!(
            "     taken by {by}: {chosen}. {}\n",
            l.text(*at, "rationale")
        )),
        DecisionState::Superseded(by) => s.push_str(&format!("     superseded by {by}\n")),
    }
    s
}

/// A mechanical decision in one line, for the report.
fn decision_line(l: &Loaded, d: &Decision) -> String {
    let chosen = match &d.state {
        DecisionState::Taken { chosen, .. } => l.text(d.at, &format!("option.{chosen}")),
        _ => "open",
    };
    format!(
        "{:<5} {}  → {}  ({}{})",
        d.id.to_string(),
        l.text(d.at, "question"),
        chosen,
        d.by,
        d.about.map(|i| format!(", {i}")).unwrap_or_default()
    )
}

fn message_json(l: &Loaded, m: &Message) -> Value {
    json!({
        "id": m.id.to_string(),
        "from": m.from.to_string(),
        "to": m.to.to_string(),
        "kind": format!("{:?}", m.kind).to_lowercase(),
        "about": m.about.map(|i| i.to_string()),
        "body": l.text(m.at, "body"),
    })
}

/// `ris ready`: what an agent may claim now.
pub fn ready(l: &Loaded) -> Output {
    let items: Vec<&Item> = l.state.ready().collect();
    let text = if items.is_empty() {
        "nothing ready".to_owned()
    } else {
        items
            .iter()
            .map(|i| item_line(l, i))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Output::new(
        text,
        json!({ "ready": items.iter().map(|i| item_json(l, i)).collect::<Vec<_>>() }),
    )
}

/// `ris inbox`: for the person, the decision queue; for an agent, its
/// messages and the items it holds.
pub fn inbox(l: &Loaded, actor: &Actor) -> Output {
    match actor {
        Actor::Person => {
            let open: Vec<&Decision> = l
                .state
                .open_decisions()
                .filter(|d| d.class == DecisionClass::Substantive)
                .collect();
            let text = if open.is_empty() {
                "no decisions waiting".to_owned()
            } else {
                let mut s = format!("{} decision(s) waiting\n", open.len());
                for d in &open {
                    s.push_str(&decision_block(l, d));
                }
                s.trim_end().to_owned()
            };
            Output::new(
                text,
                json!({ "decisions": open.iter().map(|d| decision_json(l, d)).collect::<Vec<_>>() }),
            )
        }
        Actor::Agent(name) => agent_inbox(l, name),
    }
}

fn agent_inbox(l: &Loaded, name: &AgentName) -> Output {
    let held: Vec<&Item> = l
        .state
        .items
        .values()
        .filter(|i| matches!(&i.state, ItemState::Claimed { agent } if agent == name))
        .collect();
    let messages: Vec<&Message> = l.state.messages.iter().filter(|m| &m.to == name).collect();
    let mut s = String::new();
    if held.is_empty() {
        s.push_str("holding nothing; see `ris ready`\n");
    } else {
        for i in &held {
            s.push_str(&format!("holding {}\n", item_line(l, i)));
        }
    }
    if messages.is_empty() {
        s.push_str("no messages");
    } else {
        for m in &messages {
            s.push_str(&format!(
                "{} {:?} from {}{}: {}\n",
                m.id,
                m.kind,
                m.from,
                m.about.map(|i| format!(" about {i}")).unwrap_or_default(),
                l.text(m.at, "body")
            ));
        }
    }
    Output::new(
        s.trim_end(),
        json!({
            "holding": held.iter().map(|i| item_json(l, i)).collect::<Vec<_>>(),
            "messages": messages.iter().map(|m| message_json(l, m)).collect::<Vec<_>>(),
        }),
    )
}

/// `ris report`: the phase as the person reads it. Substantive
/// decisions in full, mechanical ones one line each, items by state,
/// and the chain head.
pub fn report(l: &Loaded) -> Output {
    let Some(phase) = l.state.phases.values().last() else {
        return Output::new("no phase yet", json!({ "phase": null }));
    };
    let items: Vec<&Item> = l
        .state
        .items
        .values()
        .filter(|i| i.phase == phase.id || !matches!(i.state, ItemState::Merged { .. }))
        .collect();
    let (substantive, mechanical): (Vec<&Decision>, Vec<&Decision>) = l
        .state
        .decisions
        .values()
        .partition(|d| d.class == DecisionClass::Substantive);
    let mut s = format!(
        "{} {}: {}\n",
        phase.id,
        match phase.state {
            PhaseState::Open => "open",
            PhaseState::Closed { .. } => "closed",
        },
        l.text(phase.at, "goal")
    );
    let mut counts = std::collections::BTreeMap::new();
    for i in &items {
        *counts.entry(state_name(&i.state)).or_insert(0usize) += 1;
    }
    s.push_str(&format!(
        "items: {}\n",
        counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    for i in &items {
        s.push_str(&format!("  {}\n", item_line(l, i)));
    }
    if !substantive.is_empty() {
        s.push_str("substantive decisions:\n");
        for d in &substantive {
            for line in decision_block(l, d).lines() {
                s.push_str(&format!("  {line}\n"));
            }
        }
    }
    if !mechanical.is_empty() {
        s.push_str("mechanical decisions (check the class):\n");
        for d in &mechanical {
            s.push_str(&format!("  {}\n", decision_line(l, d)));
        }
    }
    let failed: Vec<String> = items
        .iter()
        .flat_map(|i| {
            i.check_runs
                .iter()
                .filter(|r| !r.passed)
                .map(move |r| format!("  {} {} failed ({})", i.id, r.check, r.evidence))
        })
        .collect();
    if !failed.is_empty() {
        s.push_str("failed checks:\n");
        s.push_str(&failed.join("\n"));
        s.push('\n');
    }
    s.push_str(&format!(
        "messages: {}\nlog: {} events, head {}",
        l.state.messages.len(),
        l.state.len,
        l.state.head
    ));
    Output::new(
        s,
        json!({
            "phase": {
                "id": phase.id.to_string(),
                "goal": l.text(phase.at, "goal"),
                "state": match phase.state { PhaseState::Open => "open", PhaseState::Closed { .. } => "closed" },
            },
            "items": items.iter().map(|i| item_json(l, i)).collect::<Vec<_>>(),
            "substantive": substantive.iter().map(|d| decision_json(l, d)).collect::<Vec<_>>(),
            "mechanical": mechanical.iter().map(|d| decision_json(l, d)).collect::<Vec<_>>(),
            "messages": l.state.messages.len(),
            "log": { "len": l.state.len, "head": l.state.head.to_string() },
        }),
    )
}
