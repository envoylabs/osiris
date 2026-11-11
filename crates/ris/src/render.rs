//! Rendering: what each command prints, as text and as JSON.
//!
//! Agents read this output, so the text is short and stable, and the
//! JSON carries the same facts with their ids.

use ris_core::command::report_hash;
use ris_core::ids::{Actor, AgentName, Hash, PhaseId};
use ris_core::primitives::{
    Action, ActionState, Decision, DecisionClass, DecisionState, Item, ItemState, Message,
    PhaseState,
};
use serde_json::{Value, json};

use crate::project::Loaded;
use crate::style;

/// One command's output.
pub struct Output {
    pub text: String,
    pub json: Value,
    /// `json` is an array printed as one envelope per line, as a stream
    /// is, rather than as one document.
    pub stream: bool,
}

/// The version of the `--json` contract. Every document and every
/// stream line is an envelope carrying it:
///
/// ```text
/// {"format": 1, "ok": true,  "data": ...}
/// {"format": 1, "ok": false, "error": {"kind": ..., "message": ..., "next": ...}}
/// ```
///
/// A change to the shape of any `data` or `error` is a new format.
pub const JSON_FORMAT: u32 = 1;

/// A successful result in the envelope.
pub fn envelope(data: &Value) -> Value {
    json!({ "format": JSON_FORMAT, "ok": true, "data": data })
}

impl Output {
    pub fn new(text: impl Into<String>, json: Value) -> Self {
        Self {
            text: text.into(),
            json,
            stream: false,
        }
    }

    /// Lines of text, or with `--json` one envelope per item.
    pub fn stream(text: impl Into<String>, items: Vec<Value>) -> Self {
        Self {
            text: text.into(),
            json: Value::Array(items),
            stream: true,
        }
    }
}

/// What a view covers: everything, or the open phase alone (nothing,
/// when no phase is open).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Open(Option<PhaseId>),
}

impl Scope {
    /// The open phase, or everything when `all` is asked for.
    pub fn of(l: &Loaded, all: bool) -> Self {
        if all {
            Self::All
        } else {
            Self::Open(l.state.open_phase().map(|p| p.id))
        }
    }

    pub fn item(self, i: &Item) -> bool {
        match self {
            Self::All => true,
            Self::Open(p) => Some(i.phase) == p,
        }
    }

    /// A decision belongs to the phase of the item it is about; one
    /// about no item, to the phase open when it was raised.
    pub fn decision(self, l: &Loaded, d: &Decision) -> bool {
        match self {
            Self::All => true,
            Self::Open(None) => false,
            Self::Open(Some(p)) => match d.about {
                Some(i) => l.state.items.get(&i).is_some_and(|i| self.item(i)),
                None => l.state.phases.get(&p).is_some_and(|ph| d.at >= ph.at),
            },
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
        ItemState::Superseded { .. } => "superseded",
    }
}

/// The state word the person and agents see for an item: its stored
/// state, except that a ready item with an unmerged dependency is
/// `waiting`, since `ris ready` does not offer it. The graph and the
/// report both use this, so one screen never gives an item two states.
pub(crate) fn shown_state(l: &Loaded, i: &Item) -> &'static str {
    match i.state {
        ItemState::Ready if !l.state.unlanded_dependencies(i).is_empty() => "waiting",
        _ => state_name(&i.state),
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
        "audited_by": i.audited_by.iter().map(ToString::to_string).collect::<Vec<_>>(),
    });
    match &i.state {
        ItemState::Claimed { agent } => v["agent"] = json!(agent.to_string()),
        ItemState::Blocked { on } => v["blocked_on"] = json!(on),
        ItemState::Done { evidence } => {
            v["evidence"] = json!(evidence.iter().map(ToString::to_string).collect::<Vec<_>>())
        }
        ItemState::Merged { evidence, commit } => {
            v["evidence"] = json!(evidence.iter().map(ToString::to_string).collect::<Vec<_>>());
            v["commit"] = json!(commit.to_string());
        }
        ItemState::Superseded { by } => v["superseded_by"] = json!(by.to_string()),
        ItemState::Ready => {
            let waiting = l.state.unlanded_dependencies(i);
            if !waiting.is_empty() {
                v["waiting_on"] =
                    json!(waiting.iter().map(ToString::to_string).collect::<Vec<_>>());
            }
        }
    }
    v
}

fn item_line(l: &Loaded, i: &Item) -> String {
    let who = match &i.state {
        ItemState::Claimed { agent } => format!(" ({agent})"),
        ItemState::Merged { commit, .. } => format!(" ({})", short(&commit.0)),
        ItemState::Superseded { by } => format!(" (by {by})"),
        _ => String::new(),
    };
    let state = shown_state(l, i);
    format!(
        "{:<5} {}{} {}{}",
        i.id.to_string(),
        // Styled before padding: the style matches the bare word.
        style::state(state),
        " ".repeat(8usize.saturating_sub(state.len())),
        l.text(i.at, "goal"),
        who
    )
}

fn short(s: &str) -> &str {
    &s[..s.len().min(8)]
}

/// Items merged before a person action that informs them was done:
/// work that went ahead of the person's input, for review.
fn ahead_of_the_person(l: &Loaded) -> Vec<(ris_core::ids::ItemId, ris_core::ids::ActionId)> {
    let merged_at: std::collections::BTreeMap<_, _> = l
        .log
        .iter()
        .filter_map(|e| match &e.structure {
            ris_core::event::Structure::ItemMerged { item, .. } => Some((*item, e.seq)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for a in l.state.actions.values() {
        let done_at = match a.state {
            ActionState::Done { at, .. } => at,
            ActionState::Open => u64::MAX,
        };
        for i in &a.informs {
            if merged_at.get(i).is_some_and(|m| *m < done_at) {
                out.push((*i, a.id));
            }
        }
    }
    out
}

/// A person action in full, for the inbox and `ris show`.
fn action_block(l: &Loaded, a: &Action) -> String {
    let mut s = format!(
        "{} {}  (raised by {})\n     done when: {}\n",
        a.id,
        l.text(a.at, "what"),
        a.by,
        l.text(a.at, "done_when")
    );
    let ids = |v: &[ris_core::ids::ItemId]| {
        v.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !a.blocks.is_empty() {
        s.push_str(&format!("     blocks: {}\n", ids(&a.blocks)));
    }
    if !a.informs.is_empty() {
        s.push_str(&format!("     informs: {}\n", ids(&a.informs)));
    }
    if let Some(at) = a.check {
        s.push_str(&format!(
            "     check: {} (passes when it is done)\n",
            l.text(at, "check")
        ));
    }
    match &a.state {
        ActionState::Open => s.push_str(&format!(
            "     open; when done: ris action done {} --note \"...\" or --evidence <file>\n",
            a.id
        )),
        ActionState::Done { at, evidence } => {
            s.push_str("     done");
            if let Some(n) = l.log_text(*at, "note") {
                s.push_str(&format!(": {n}"));
            }
            if !evidence.is_empty() {
                s.push_str(&format!(
                    " (evidence {})",
                    evidence
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            s.push('\n');
        }
    }
    s
}

fn action_json(l: &Loaded, a: &Action) -> Value {
    let mut v = json!({
        "id": a.id.to_string(),
        "by": a.by,
        "what": l.text(a.at, "what"),
        "done_when": l.text(a.at, "done_when"),
        "blocks": a.blocks.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "informs": a.informs.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "state": match a.state { ActionState::Open => "open", ActionState::Done { .. } => "done" },
        "check": a.check.map(|at| l.text(at, "check")),
    });
    if let ActionState::Done { at, evidence } = &a.state {
        v["note"] = json!(l.log_text(*at, "note"));
        v["evidence"] = json!(evidence.iter().map(ToString::to_string).collect::<Vec<_>>());
    }
    v
}

fn decision_json(l: &Loaded, d: &Decision) -> Value {
    let options: Vec<&str> = (0..d.options)
        .map(|n| l.text(d.at, &format!("option.{n}")))
        .collect();
    let mut v = json!({
        "id": d.id.to_string(),
        "class": match d.class { DecisionClass::Mechanical => "mechanical", DecisionClass::Substantive => "substantive" },
        "by": d.by,
        "about": d.about.map(|i| i.to_string()),
        "question": l.text(d.at, "question"),
        "options": options,
        "recommended": d.recommended,
        "state": match d.state { DecisionState::Open => "open", DecisionState::Taken { .. } => "taken", DecisionState::Superseded(_) => "superseded" },
        "explained": d.explained.is_some(),
    });
    if let DecisionState::Taken { by, chosen, at } = &d.state {
        v["taken_by"] = json!(by);
        v["chosen"] = json!(chosen);
        v["rationale"] = json!(l.text(*at, "rationale"));
        if *chosen >= d.options {
            v["other"] = json!(l.text(*at, "other"));
        }
    }
    v
}

/// `ris eli5`: the full case for a decision. The question, then each
/// option with its points for and against, the recommendation and why,
/// and how to answer. Without an explanation, says so and who can give
/// one.
pub fn eli5(l: &Loaded, d: &Decision) -> Output {
    let mut out = eli5_case(l, d);
    if d.explained.is_none() {
        out.text.push_str(&format!(
            "  next: ask {} for one (`ris decision explain {}`), or decide from the above\n",
            match &d.by {
                Actor::Person => "yourself".to_owned(),
                Actor::Agent(a) => a.to_string(),
            },
            d.id
        ));
        return out;
    }
    match &d.state {
        DecisionState::Open => out.text.push_str(&format!(
            "--\nanswer:  ris decide {0} --choose <option number> --rationale \"...\"\n     or  ris decide {0} --other \"your own answer\" --rationale \"...\"",
            d.id
        )),
        _ => {
            out.text.push_str("--\n");
            out.text
                .push_str(decision_block(l, d).lines().last().unwrap_or("").trim());
        }
    }
    out
}

/// A decision's case and nothing else: what `ris eli5` prints without
/// the lines on how to answer, which `ris decide` shows before its
/// prompt. Without an explanation, the decision and a line saying none
/// is recorded.
pub fn eli5_case(l: &Loaded, d: &Decision) -> Output {
    let class = match d.class {
        DecisionClass::Mechanical => "mechanical",
        DecisionClass::Substantive => "substantive",
    };
    let mut s = format!("{} [{class}] {}\n", d.id, l.text(d.at, "question"));
    if let Some(i) = d.about {
        s.push_str(&format!("about {i}; raised by {}\n", d.by));
    } else {
        s.push_str(&format!("raised by {}\n", d.by));
    }
    let Some(e) = &d.explained else {
        s.push('\n');
        s.push_str(&decision_block(l, d));
        s.push_str("\nno explanation recorded\n");
        return Output::new(
            s,
            json!({ "decision": decision_json(l, d), "explanation": null }),
        );
    };
    s.push_str(&format!("\n{}\n", l.text(e.at, "summary")));
    let mut options = Vec::new();
    for n in 0..d.options {
        let pros: Vec<&str> = (0..e.pros[n])
            .map(|k| l.text(e.at, &format!("option.{n}.pro.{k}")))
            .collect();
        let cons: Vec<&str> = (0..e.cons[n])
            .map(|k| l.text(e.at, &format!("option.{n}.con.{k}")))
            .collect();
        let mark = if n == d.recommended {
            style::recommended("* recommended")
        } else {
            String::new()
        };
        s.push_str(&format!(
            "\n{n}: {}  {mark}\n",
            l.text(d.at, &format!("option.{n}"))
        ));
        for p in &pros {
            s.push_str(&format!("   + {p}\n"));
        }
        for c in &cons {
            s.push_str(&format!("   - {c}\n"));
        }
        options.push(json!({ "option": n, "text": l.text(d.at, &format!("option.{n}")), "pros": pros, "cons": cons }));
    }
    s.push_str(&format!(
        "\nwhy {}: {}\n",
        d.recommended,
        l.text(e.at, "why")
    ));
    s.push_str(&format!("explained by {}\n", e.by));
    Output::new(
        s,
        json!({
            "decision": decision_json(l, d),
            "explanation": {
                "by": e.by,
                "summary": l.text(e.at, "summary"),
                "options": options,
                "why": l.text(e.at, "why"),
            }
        }),
    )
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
        let mark = if n == d.recommended {
            style::recommended("*")
        } else {
            " ".to_owned()
        };
        s.push_str(&format!(
            "   {mark} {n}: {}\n",
            l.text(d.at, &format!("option.{n}"))
        ));
    }
    match &d.state {
        DecisionState::Open if d.explained.is_some() => s.push_str(&format!(
            "     open; * marks the recommendation; the full case: ris eli5 {}\n",
            d.id
        )),
        DecisionState::Open => s.push_str("     open; * marks the recommendation\n"),
        DecisionState::Taken { by, chosen, at } if *chosen >= d.options => {
            s.push_str(&format!(
                "   {} {chosen}: {}\n     taken by {by}: {chosen}, outside the offered options. {}\n",
                style::other("+"),
                l.text(*at, "other"),
                l.text(*at, "rationale")
            ));
        }
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
        DecisionState::Taken { chosen, at, .. } if *chosen >= d.options => l.text(*at, "other"),
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
        "from": m.from,
        "to": m.to,
        "kind": format!("{:?}", m.kind).to_lowercase(),
        "about": m.about.map(|i| i.to_string()),
        "body": l.text(m.at, "body"),
    })
}

/// `ris ready`: what an agent may claim now, the open phase's own items
/// first, then items carried over from earlier phases, each group in id
/// order. A seat takes the first, so the phase's work comes before the
/// backlog.
pub fn ready(l: &Loaded) -> Output {
    ready_in(l, Scope::All)
}

/// What may be claimed now, within `scope`.
fn ready_in(l: &Loaded, scope: Scope) -> Output {
    let open = l.state.open_phase().map(|p| p.id);
    let mut items: Vec<&Item> = l.state.ready().filter(|i| scope.item(i)).collect();
    items.sort_by_key(|i| Some(i.phase) != open);
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

/// What the person's inbox ends with: how to act, and what the ids mean.
const PERSON_FOOTER: &str = "\
--
answer:  ris decide D-n --choose <option number> --rationale \"...\"
     or  ris decide D-n --other \"your own answer\" --rationale \"...\"
   and   ris action done A-n --note \"what happened\" (or --evidence <file>)
look:    ris eli5 D-n (a decision's full case)    ris show <id>    ris ready    ris report
ids:     I-n item   D-n decision   A-n action   P-n phase   M-n message   E-<hash> evidence
";

/// `ris inbox`: for the person, the decision queue; for an agent, its
/// messages and the items it holds.
pub fn inbox(l: &Loaded, actor: &Actor) -> Output {
    inbox_in(l, actor, Scope::All)
}

/// The inbox with the person's decisions limited to `scope`. Messages,
/// held items and person actions are not phase work and always show.
fn inbox_in(l: &Loaded, actor: &Actor, scope: Scope) -> Output {
    match actor {
        Actor::Person => {
            let open: Vec<&Decision> = l
                .state
                .open_decisions()
                .filter(|d| d.class == DecisionClass::Substantive && scope.decision(l, d))
                .collect();
            let mut text = if open.is_empty() {
                "no decisions waiting\n".to_owned()
            } else {
                let mut s = style::heading(&format!("{} decision(s) waiting", open.len())) + "\n";
                for d in &open {
                    s.push_str(&decision_block(l, d));
                }
                s
            };
            let actions: Vec<&Action> = l
                .state
                .actions
                .values()
                .filter(|a| a.state == ActionState::Open)
                .collect();
            if !actions.is_empty() {
                text.push_str(&format!(
                    "\n{}\n",
                    style::heading(&format!("{} action(s) for you", actions.len()))
                ));
                for a in &actions {
                    text.push_str(&action_block(l, a));
                }
            }
            text.push_str(&style::dim(PERSON_FOOTER));
            let text = text.trim_end().to_owned();
            Output::new(
                text,
                json!({
                    "decisions": open.iter().map(|d| decision_json(l, d)).collect::<Vec<_>>(),
                    "actions": actions.iter().map(|a| action_json(l, a)).collect::<Vec<_>>(),
                }),
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
    report_in(l, Scope::All)
}

/// The report with its items and decisions limited to `scope`. Under
/// [`Scope::All`] it lists the latest phase's items and every item
/// still in motion from earlier ones, and every decision.
fn report_in(l: &Loaded, scope: Scope) -> Output {
    let Some(phase) = l.state.phases.values().last() else {
        return Output::new("no phase yet", json!({ "phase": null }));
    };
    let items: Vec<&Item> = l
        .state
        .items
        .values()
        .filter(|i| {
            scope.item(i)
                && (i.phase == phase.id
                    || !matches!(
                        i.state,
                        ItemState::Merged { .. } | ItemState::Superseded { .. }
                    ))
        })
        .collect();
    let (substantive, mechanical): (Vec<&Decision>, Vec<&Decision>) = l
        .state
        .decisions
        .values()
        .filter(|d| scope.decision(l, d))
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
        *counts.entry(shown_state(l, i)).or_insert(0usize) += 1;
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
    let outside = l
        .state
        .decisions
        .values()
        .filter(|d| scope.decision(l, d))
        .filter(|d| matches!(&d.state, DecisionState::Taken { chosen, .. } if *chosen >= d.options))
        .count();
    if !substantive.is_empty() {
        s.push_str(&style::heading(&format!(
            "substantive decisions ({outside} answered outside the offered options):"
        )));
        s.push('\n');
        for d in &substantive {
            for line in decision_block(l, d).lines() {
                s.push_str(&format!("  {line}\n"));
            }
        }
    }
    if !mechanical.is_empty() {
        s.push_str(&style::heading("mechanical decisions (check the class):"));
        s.push('\n');
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
        s.push_str(&style::heading("failed checks:"));
        s.push('\n');
        s.push_str(&failed.join("\n"));
        s.push('\n');
    }
    // The report's hash is taken over the previous report and the log
    // head, never over this rendering, so removing text later does not
    // break the chain of reports.
    let report_hash = match phase.state {
        PhaseState::Closed { report } => report,
        PhaseState::Open => report_hash(&l.state.last_report, &l.state.head),
    };
    let finished: Vec<&&Item> = items
        .iter()
        .filter(|i| matches!(i.state, ItemState::Done { .. } | ItemState::Merged { .. }))
        .collect();
    let audited = finished.iter().filter(|i| !i.audited_by.is_empty()).count();
    let ahead = ahead_of_the_person(l);
    if !ahead.is_empty() {
        s.push_str(&style::heading("merged ahead of the person:"));
        s.push('\n');
        for (item, action) in &ahead {
            s.push_str(&format!(
                "  {item} merged before {action} was done: {}\n",
                l.text(l.state.actions[action].at, "what")
            ));
        }
    }
    s.push_str(&format!(
        "audited: {audited} of {} done or merged\n",
        finished.len()
    ));
    s.push_str(&format!(
        "messages: {}\nlog: {} events, head {}\nreport: {}{}\nprevious report: {}",
        l.state.messages.len(),
        l.state.len,
        l.state.head,
        report_hash,
        if matches!(phase.state, PhaseState::Open) {
            " (if closed now)"
        } else {
            ""
        },
        previous_report(l, phase.id)
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
            "answered_outside_options": outside,
            "merged_ahead": ahead
                .iter()
                .map(|(i, a)| json!({ "item": i.to_string(), "action": a.to_string() }))
                .collect::<Vec<_>>(),
            "log": { "len": l.state.len, "head": l.state.head.to_string() },
            "report": report_hash.to_string(),
            "previous_report": previous_report(l, phase.id).to_string(),
        }),
    )
}

/// The report hash of the phase closed before `phase`, or zero.
fn previous_report(l: &Loaded, phase: PhaseId) -> Hash {
    l.state
        .phases
        .values()
        .filter(|p| p.id < phase)
        .filter_map(|p| match p.state {
            PhaseState::Closed { report } => Some(report),
            PhaseState::Open => None,
        })
        .next_back()
        .unwrap_or(Hash::ZERO)
}

/// `ris show <id>`: one record in full. `id` is an item, decision,
/// phase or message id; anything else is an error naming the forms.
pub fn show(l: &Loaded, id: &str) -> Result<Output, String> {
    if let Ok(item) = id.parse::<ris_core::ids::ItemId>() {
        let i = l
            .state
            .items
            .get(&item)
            .ok_or_else(|| format!("no item {item}"))?;
        return Ok(show_item(l, i));
    }
    if let Ok(decision) = id.parse::<ris_core::ids::DecisionId>() {
        let d = l
            .state
            .decisions
            .get(&decision)
            .ok_or_else(|| format!("no decision {decision}"))?;
        let mut json = decision_json(l, d);
        json["raised_at_event"] = json!(d.at);
        return Ok(Output::new(decision_block(l, d).trim_end(), json));
    }
    if let Ok(action) = id.parse::<ris_core::ids::ActionId>() {
        let a = l
            .state
            .actions
            .get(&action)
            .ok_or_else(|| format!("no action {action}"))?;
        return Ok(Output::new(
            action_block(l, a).trim_end(),
            action_json(l, a),
        ));
    }
    if let Ok(phase) = id.parse::<PhaseId>() {
        let p = l
            .state
            .phases
            .get(&phase)
            .ok_or_else(|| format!("no phase {phase}"))?;
        let items: Vec<&Item> = l
            .state
            .items
            .values()
            .filter(|i| i.phase == phase)
            .collect();
        let mut s = format!(
            "{} {}: {}\n",
            p.id,
            match p.state {
                PhaseState::Open => "open",
                PhaseState::Closed { .. } => "closed",
            },
            l.text(p.at, "goal")
        );
        for i in &items {
            s.push_str(&format!("  {}\n", item_line(l, i)));
        }
        if let PhaseState::Closed { report } = p.state {
            s.push_str(&format!("report {report}\n"));
        }
        return Ok(Output::new(
            s.trim_end(),
            json!({
                "id": p.id.to_string(),
                "goal": l.text(p.at, "goal"),
                "state": match p.state { PhaseState::Open => "open", PhaseState::Closed { .. } => "closed" },
                "items": items.iter().map(|i| item_json(l, i)).collect::<Vec<_>>(),
            }),
        ));
    }
    if let Ok(message) = id.parse::<ris_core::ids::MessageId>() {
        let m = l
            .state
            .messages
            .iter()
            .find(|m| m.id == message)
            .ok_or_else(|| format!("no message {message}"))?;
        return Ok(Output::new(
            format!(
                "{} {:?} from {} to {}{}\n  {}",
                m.id,
                m.kind,
                m.from,
                m.to,
                m.about.map(|i| format!(" about {i}")).unwrap_or_default(),
                l.text(m.at, "body")
            ),
            message_json(l, m),
        ));
    }
    Err(format!(
        "`{id}` is not an id; use I-n (item), D-n (decision), P-n (phase), M-n (message) or E-<hash> (evidence); the hyphen is optional"
    ))
}

fn show_item(l: &Loaded, i: &Item) -> Output {
    let mut s = format!(
        "{} {}  (phase {})\n  {}\n",
        i.id,
        shown_state(l, i),
        i.phase,
        l.text(i.at, "goal")
    );
    s.push_str(&format!(
        "  checks: {}\n",
        if i.checks.is_empty() {
            "none".to_owned()
        } else {
            i.checks
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    if !i.depends_on.is_empty() {
        let waiting = l.state.unlanded_dependencies(i);
        s.push_str(&format!(
            "  depends on: {}{}\n",
            i.depends_on
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            if waiting.is_empty() {
                " (all merged)".to_owned()
            } else {
                format!(
                    " (waiting on {})",
                    waiting
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ));
    }
    match &i.state {
        ItemState::Claimed { agent } => s.push_str(&format!("  held by: {agent}\n")),
        ItemState::Blocked { on } => s.push_str(&format!(
            "  blocked on: {}\n",
            on.iter()
                .map(|b| match b {
                    ris_core::primitives::Blocker::Decision(d) => d.to_string(),
                    ris_core::primitives::Blocker::Item(i) => i.to_string(),
                    ris_core::primitives::Blocker::Action(a) => a.to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        )),
        ItemState::Done { evidence } | ItemState::Merged { evidence, .. } => {
            s.push_str(&format!(
                "  evidence: {}\n",
                evidence
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            if let ItemState::Merged { commit, .. } = &i.state {
                s.push_str(&format!("  commit: {commit}\n"));
            }
        }
        ItemState::Superseded { by } => s.push_str(&format!("  superseded by: {by}\n")),
        ItemState::Ready => {}
    }
    if !i.audited_by.is_empty() {
        s.push_str(&format!(
            "  audited by: {}\n",
            i.audited_by
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let decisions: Vec<&Decision> = l
        .state
        .decisions
        .values()
        .filter(|d| d.about == Some(i.id))
        .collect();
    if !decisions.is_empty() {
        s.push_str("  decisions:\n");
        for d in &decisions {
            s.push_str(&format!("    {}\n", decision_line(l, d)));
        }
    }
    if !i.check_runs.is_empty() {
        s.push_str("  check runs:\n");
        for r in &i.check_runs {
            s.push_str(&format!(
                "    {} {} ({})\n",
                r.check,
                if r.passed {
                    style::passed("passed")
                } else {
                    style::failed("FAILED")
                },
                r.evidence
            ));
        }
    }
    let mut json = item_json(l, i);
    json["decisions"] = json!(
        decisions
            .iter()
            .map(|d| decision_json(l, d))
            .collect::<Vec<_>>()
    );
    json["check_runs"] = json!(
        i.check_runs
            .iter()
            .map(|r| json!({ "check": r.check.to_string(), "passed": r.passed, "evidence": r.evidence.to_string() }))
            .collect::<Vec<_>>()
    );
    Output::new(s.trim_end(), json)
}

/// `ris summary`: the check-in view. The decision queue first, since
/// it is what blocks agents; then the graph, the report and what is
/// ready to claim. By default each section covers the open phase alone,
/// and what it leaves out from earlier phases is counted; `all` shows
/// everything, each section then the same as its own command.
pub fn summary(l: &Loaded, actor: &Actor, all: bool) -> Output {
    let scope = Scope::of(l, all);
    let queue = inbox_in(l, actor, scope);
    let rep = report_in(l, scope);
    let rdy = ready_in(l, scope);
    let graph = crate::graph::Graph::of_scope(l, scope);
    // Unsettled work outside the scope: items not merged or superseded,
    // and open decisions.
    let carried_items = l
        .state
        .items
        .values()
        .filter(|i| {
            !scope.item(i)
                && !matches!(
                    i.state,
                    ItemState::Merged { .. } | ItemState::Superseded { .. }
                )
        })
        .count();
    let carried_decisions = l
        .state
        .open_decisions()
        .filter(|d| !scope.decision(l, d))
        .count();
    let mut head = String::new();
    if scope == Scope::Open(None) {
        head.push_str(
            "no phase is open\n  next: open one with `ris phase open \"<goal>\"`, then bring items into it\n",
        );
    }
    if carried_items + carried_decisions > 0 {
        head.push_str(&format!(
            "{carried_items} item(s) and {carried_decisions} open decision(s) from earlier phases are not shown\n  \
             next: `ris summary --all` shows them\n"
        ));
    }
    if !head.is_empty() {
        head.push('\n');
    }
    let text = format!(
        "{head}{}\n{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}",
        style::header("== decisions =="),
        queue.text,
        style::header("== what waits on what =="),
        graph.draw(l),
        style::header("== report =="),
        rep.text,
        style::header("== ready =="),
        rdy.text
    );
    Output::new(
        text,
        json!({
            "inbox": queue.json,
            "graph": graph.json(l),
            "report": rep.json,
            "ready": rdy.json,
            "all": all,
            "carried_over": { "items": carried_items, "decisions": carried_decisions },
        }),
    )
}

/// `ris audit`: the review queue, limited to `scope` (the open phase's
/// items unless `--all` is asked for). For an agent, done and merged
/// items it did not author and has not reviewed; for the person, those
/// nobody has reviewed.
pub fn audit_queue(l: &Loaded, actor: &Actor, scope: Scope) -> Output {
    let queue: Vec<&Item> = l
        .state
        .items
        .values()
        .filter(|i| scope.item(i))
        .filter(|i| matches!(i.state, ItemState::Done { .. } | ItemState::Merged { .. }))
        .filter(|i| match actor {
            Actor::Agent(me) => i.author.as_ref() != Some(me) && !i.audited_by.contains(me),
            Actor::Person => i.audited_by.is_empty(),
        })
        .collect();
    let text = if queue.is_empty() {
        "nothing to review".to_owned()
    } else {
        let mut s = queue
            .iter()
            .map(|i| item_line(l, i))
            .collect::<Vec<_>>()
            .join("\n");
        s.push_str("\n--\nreview: ris show I-n, the diff, the evidence; findings: ris send <author> --kind finding --about I-n \"...\"; then ris audit I-n");
        s
    };
    Output::new(
        text,
        json!({ "queue": queue.iter().map(|i| item_json(l, i)).collect::<Vec<_>>() }),
    )
}
