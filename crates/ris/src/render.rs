//! Rendering: what each command prints, as text and as JSON.
//!
//! Agents read this output, so the text is short and stable, and the
//! JSON carries the same facts with their ids.

use ris_core::command::report_hash;
use ris_core::ids::{Actor, AgentName, DecisionId, Hash, PhaseId};
use ris_core::primitives::{
    Action, ActionState, Decision, DecisionClass, DecisionState, Item, ItemState, Message,
    PhaseState,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use crate::project::Loaded;
use crate::schema;
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

/// A successful result in the envelope. `data` is a type with a
/// schema, as every command's is (`crate::schema`).
pub fn envelope<T: Serialize + JsonSchema + ?Sized>(data: &T) -> Value {
    json!({ "format": JSON_FORMAT, "ok": true, "data": data })
}

impl Output {
    /// A command's output: its text, and its data for `--json`. The
    /// data is a type the schema is generated from, so the two cannot
    /// drift; there is no way to build an output from loose JSON.
    pub fn of<T: Serialize + JsonSchema + ?Sized>(text: impl Into<String>, data: &T) -> Self {
        Self {
            text: text.into(),
            json: serde_json::to_value(data).expect("output data serialises"),
            stream: false,
        }
    }

    /// Lines of text, or with `--json` one envelope per item.
    pub fn stream<T: Serialize + JsonSchema>(text: impl Into<String>, items: &[T]) -> Self {
        Self {
            text: text.into(),
            json: serde_json::to_value(items).expect("output data serialises"),
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

fn item_json(l: &Loaded, i: &Item) -> schema::Item {
    let mut v = schema::Item {
        id: i.id,
        phase: i.phase,
        goal: l.text(i.at, "goal").to_owned(),
        state: state_name(&i.state),
        checks: i.checks.clone(),
        depends_on: i.depends_on.clone(),
        audited_by: i.audited_by.clone(),
        agent: None,
        blocked_on: None,
        evidence: None,
        commit: None,
        superseded_by: None,
        waiting_on: None,
        decisions: None,
        check_runs: None,
    };
    match &i.state {
        ItemState::Claimed { agent } => v.agent = Some(agent.clone()),
        ItemState::Blocked { on } => v.blocked_on = Some(on.clone()),
        ItemState::Done { evidence } => v.evidence = Some(evidence.clone()),
        ItemState::Merged { evidence, commit } => {
            v.evidence = Some(evidence.clone());
            v.commit = Some(commit.clone());
        }
        ItemState::Superseded { by } => v.superseded_by = Some(*by),
        ItemState::Ready => {
            let waiting = l.state.unlanded_dependencies(i);
            if !waiting.is_empty() {
                v.waiting_on = Some(waiting);
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

fn action_json(l: &Loaded, a: &Action) -> schema::Action {
    let mut v = schema::Action {
        id: a.id,
        by: a.by.clone(),
        what: l.text(a.at, "what").to_owned(),
        done_when: l.text(a.at, "done_when").to_owned(),
        blocks: a.blocks.clone(),
        informs: a.informs.clone(),
        state: match a.state {
            ActionState::Open => "open",
            ActionState::Done { .. } => "done",
        },
        check: a.check.map(|at| l.text(at, "check").to_owned()),
        note: None,
        evidence: None,
    };
    if let ActionState::Done { at, evidence } = &a.state {
        v.note = Some(l.log_text(*at, "note").map(str::to_owned));
        v.evidence = Some(evidence.clone());
    }
    v
}

fn decision_json(l: &Loaded, d: &Decision) -> schema::Decision {
    let mut v = schema::Decision {
        id: d.id,
        class: d.class,
        by: d.by.clone(),
        about: d.about,
        question: l.text(d.at, "question").to_owned(),
        options: (0..d.options)
            .map(|n| l.text(d.at, &format!("option.{n}")).to_owned())
            .collect(),
        recommended: d.recommended,
        state: match d.state {
            DecisionState::Open => "open",
            DecisionState::Taken { .. } => "taken",
            DecisionState::Superseded(_) => "superseded",
        },
        explained: d.explained.is_some(),
        taken_by: None,
        chosen: None,
        rationale: None,
        other: None,
        raised_at_event: None,
    };
    if let DecisionState::Taken { by, chosen, at } = &d.state {
        v.taken_by = Some(by.clone());
        v.chosen = Some(*chosen);
        v.rationale = Some(l.text(*at, "rationale").to_owned());
        if *chosen >= d.options {
            v.other = Some(l.text(*at, "other").to_owned());
        }
    }
    v
}

/// `ris eli5`: the full case for a decision. The question, then each
/// option with its points for and against, the recommendation and why,
/// and how to answer. Without an explanation, says so and who can give
/// one.
pub fn eli5(l: &Loaded, d: &Decision) -> Output {
    let (mut text, case) = eli5_case(l, d);
    let done = |text: String| Output::of(text, &case);
    if d.explained.is_none() {
        text.push_str(&format!(
            "  next: ask {} for one (`ris decision explain {}`), or decide from the above\n",
            match &d.by {
                Actor::Person => "yourself".to_owned(),
                Actor::Agent(a) => a.to_string(),
            },
            d.id
        ));
        return done(text);
    }
    match &d.state {
        DecisionState::Open => text.push_str(&format!(
            "--\nanswer:  ris decide {0} --choose <option number> --rationale \"...\"\n     or  ris decide {0} --other \"your own answer\" --rationale \"...\"",
            d.id
        )),
        _ => {
            text.push_str("--\n");
            text.push_str(decision_block(l, d).lines().last().unwrap_or("").trim());
        }
    }
    done(text)
}

/// A decision's case and nothing else, as text and as data: what
/// `ris eli5` prints without the lines on how to answer, which
/// `ris decide` shows before its prompt. Without an explanation, the
/// decision and a line saying none is recorded.
pub fn eli5_case(l: &Loaded, d: &Decision) -> (String, schema::Case) {
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
        return (
            s,
            schema::Case {
                decision: decision_json(l, d),
                explanation: None,
            },
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
        options.push(schema::ExplainedOption {
            option: n,
            text: l.text(d.at, &format!("option.{n}")).to_owned(),
            pros: pros.iter().map(|p| (*p).to_owned()).collect(),
            cons: cons.iter().map(|c| (*c).to_owned()).collect(),
        });
    }
    s.push_str(&format!(
        "\nwhy {}: {}\n",
        d.recommended,
        l.text(e.at, "why")
    ));
    s.push_str(&format!("explained by {}\n", e.by));
    (
        s,
        schema::Case {
            decision: decision_json(l, d),
            explanation: Some(schema::Explanation {
                by: e.by.clone(),
                summary: l.text(e.at, "summary").to_owned(),
                options,
                why: l.text(e.at, "why").to_owned(),
            }),
        },
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

fn message_json(l: &Loaded, m: &Message) -> schema::Message {
    schema::Message {
        id: m.id,
        from: m.from.clone(),
        to: m.to.clone(),
        kind: format!("{:?}", m.kind).to_lowercase(),
        about: m.about,
        body: l.text(m.at, "body").to_owned(),
    }
}

/// `ris ready`: what may be claimed now, in id order. A seat sees only
/// the open phase's items, since it may claim no other. The person sees
/// those first and then the backlog, the items carried over from
/// earlier phases, which they bring in with `ris item bring-in`.
pub fn ready(l: &Loaded, actor: &Actor) -> Output {
    let scope = match actor {
        Actor::Person => Scope::All,
        Actor::Agent(_) => Scope::of(l, false),
    };
    let (text, data) = ready_in(l, scope);
    Output::of(text, &data)
}

/// What may be claimed now, within `scope`.
fn ready_in(l: &Loaded, scope: Scope) -> (String, schema::Ready) {
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
    (
        text,
        schema::Ready {
            ready: items.iter().map(|i| item_json(l, i)).collect(),
        },
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
///
/// `checks` is what running the person's action checks did before
/// this, which goes ahead of the text and into the data.
pub fn inbox(l: &Loaded, actor: &Actor, checks: Vec<String>) -> Output {
    let (text, mut data) = inbox_in(l, actor, Scope::All);
    let text = with_checks(text, &checks);
    match &mut data {
        schema::Inbox::Person { checks: c, .. } | schema::Inbox::Seat { checks: c, .. } => {
            *c = Some(checks);
        }
    }
    Output::of(text, &data)
}

/// `text` with what the person's action checks did ahead of it.
fn with_checks(text: String, checks: &[String]) -> String {
    if checks.is_empty() {
        text
    } else {
        format!("{}\n\n{text}", checks.join("\n"))
    }
}

/// The inbox with the person's decisions limited to `scope`. Messages,
/// held items and person actions are not phase work and always show.
fn inbox_in(l: &Loaded, actor: &Actor, scope: Scope) -> (String, schema::Inbox) {
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
            (
                text,
                schema::Inbox::Person {
                    decisions: open.iter().map(|d| decision_json(l, d)).collect(),
                    actions: actions.iter().map(|a| action_json(l, a)).collect(),
                    checks: None,
                },
            )
        }
        Actor::Agent(name) => agent_inbox(l, name),
    }
}

fn agent_inbox(l: &Loaded, name: &AgentName) -> (String, schema::Inbox) {
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
    (
        s.trim_end().to_owned(),
        schema::Inbox::Seat {
            holding: held.iter().map(|i| item_json(l, i)).collect(),
            messages: messages.iter().map(|m| message_json(l, m)).collect(),
            checks: None,
        },
    )
}

/// `ris report`: the phase as the person reads it. Substantive
/// decisions in full, mechanical ones one line each, items by state,
/// and the chain head.
pub fn report(l: &Loaded) -> Output {
    let (text, data) = report_data(l);
    Output::of(text, &data)
}

/// The report as text and as data, for a command that carries it
/// inside its own (`ris phase close`).
pub fn report_data(l: &Loaded) -> (String, schema::Report) {
    report_in(l, Scope::All)
}

/// The report with its items and decisions limited to `scope`. Under
/// [`Scope::All`] it lists the latest phase's items and every item
/// still in motion from earlier ones, and every decision.
fn report_in(l: &Loaded, scope: Scope) -> (String, schema::Report) {
    let Some(phase) = l.state.phases.values().last() else {
        return (
            "no phase yet".to_owned(),
            schema::Report::NoPhase { phase: () },
        );
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
    let open = matches!(phase.state, PhaseState::Open);
    let t = tally(l, phase.id);
    s.push_str(&format!(
        "{}: {} planned, {} merged, {} {}\n",
        phase.id,
        t.planned,
        t.merged,
        t.carried_over,
        if open {
            "carry over if closed now"
        } else {
            "carried over"
        }
    ));
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
    (
        s,
        schema::Report::Phase(Box::new(schema::PhaseReport {
            phase: schema::PhaseHead {
                id: phase.id,
                goal: l.text(phase.at, "goal").to_owned(),
                state: match phase.state {
                    PhaseState::Open => "open",
                    PhaseState::Closed { .. } => "closed",
                },
            },
            items: items.iter().map(|i| item_json(l, i)).collect(),
            tally: t,
            substantive: substantive.iter().map(|d| decision_json(l, d)).collect(),
            mechanical: mechanical.iter().map(|d| decision_json(l, d)).collect(),
            messages: l.state.messages.len(),
            answered_outside_options: outside,
            merged_ahead: ahead
                .iter()
                .map(|(item, action)| schema::MergedAhead {
                    item: *item,
                    action: *action,
                })
                .collect(),
            log: schema::LogHead {
                len: l.state.len,
                head: l.state.head,
            },
            report: report_hash,
            previous_report: previous_report(l, phase.id),
        })),
    )
}

/// What a phase came to: the items planned in it, the merges made while
/// it was open (an item carried in from an earlier phase counts where
/// it merged), and the items left unsettled, neither merged nor
/// superseded, which the next phase inherits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Tally {
    pub planned: usize,
    pub merged: usize,
    pub carried_over: usize,
}

/// The tally of `phase`, from the log: its merges are the `ItemMerged`
/// events between its opening and its close (or the head, while open).
pub fn tally(l: &Loaded, phase: PhaseId) -> Tally {
    use ris_core::event::Structure;
    let opened = l.state.phases.get(&phase).map_or(u64::MAX, |p| p.at);
    let closed = l
        .log
        .iter()
        .find_map(|e| match e.structure {
            Structure::PhaseClosed { phase: p, .. } if p == phase => Some(e.seq),
            _ => None,
        })
        .unwrap_or(u64::MAX);
    let merged = l
        .log
        .iter()
        .filter(|e| e.seq > opened && e.seq < closed)
        .filter(|e| matches!(e.structure, Structure::ItemMerged { .. }))
        .count();
    // By the phase each item was added in, which never changes, so a
    // closed phase's tally stays what it was when an item it left is
    // later brought into another phase.
    let items = l.state.items.values().filter(|i| i.added_in <= phase);
    let (mut planned, mut carried_over) = (0, 0);
    for i in items {
        planned += usize::from(i.added_in == phase);
        carried_over += usize::from(!matches!(
            i.state,
            ItemState::Merged { .. } | ItemState::Superseded { .. }
        ));
    }
    Tally {
        planned,
        merged,
        carried_over,
    }
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
        let mut data = decision_json(l, d);
        data.raised_at_event = Some(d.at);
        return Ok(Output::of(
            decision_block(l, d).trim_end(),
            &schema::Show::Decision(Box::new(data)),
        ));
    }
    if let Ok(action) = id.parse::<ris_core::ids::ActionId>() {
        let a = l
            .state
            .actions
            .get(&action)
            .ok_or_else(|| format!("no action {action}"))?;
        return Ok(Output::of(
            action_block(l, a).trim_end(),
            &schema::Show::Action(action_json(l, a)),
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
        return Ok(Output::of(
            s.trim_end(),
            &schema::Show::Phase(schema::Phase {
                id: p.id,
                goal: l.text(p.at, "goal").to_owned(),
                state: match p.state {
                    PhaseState::Open => "open",
                    PhaseState::Closed { .. } => "closed",
                },
                items: items.iter().map(|i| item_json(l, i)).collect(),
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
        return Ok(Output::of(
            format!(
                "{} {:?} from {} to {}{}\n  {}",
                m.id,
                m.kind,
                m.from,
                m.to,
                m.about.map(|i| format!(" about {i}")).unwrap_or_default(),
                l.text(m.at, "body")
            ),
            &schema::Show::Message(message_json(l, m)),
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
    let mut data = item_json(l, i);
    data.decisions = Some(decisions.iter().map(|d| decision_json(l, d)).collect());
    data.check_runs = Some(
        i.check_runs
            .iter()
            .map(|r| schema::CheckRun {
                check: r.check.clone(),
                passed: r.passed,
                evidence: r.evidence,
            })
            .collect(),
    );
    Output::of(s.trim_end(), &schema::Show::Item(Box::new(data)))
}

/// The open phase's state, counted, and said in one line: what
/// `ris summary` opens with. `running` is the seats whose agent is
/// running now. `None` with no phase open.
///
/// Every item of the phase that is not superseded is in exactly one of
/// `merged`, `in_progress` (claimed, or done and not yet merged),
/// `set_aside` (blocked) and `not_started`, so the four add up to
/// `items` and can be checked against the item list.
pub fn phase_state(l: &Loaded, running: &[AgentName]) -> Option<schema::PhaseState> {
    let phase = l.state.open_phase()?.id;
    let scope = Scope::Open(Some(phase));
    let (mut items, mut merged, mut not_started) = (0, 0, 0);
    let (mut in_progress, mut set_aside) = (Vec::new(), Vec::new());
    for i in l.state.items.values().filter(|i| scope.item(i)) {
        match &i.state {
            ItemState::Superseded { .. } => continue,
            ItemState::Merged { .. } => merged += 1,
            ItemState::Ready => not_started += 1,
            ItemState::Claimed { agent } => in_progress.push(schema::InProgress {
                item: i.id,
                seat: Some(agent.clone()),
            }),
            ItemState::Done { .. } => in_progress.push(schema::InProgress {
                item: i.id,
                seat: i.author.clone(),
            }),
            ItemState::Blocked { .. } => set_aside.push(i.id),
        }
        items += 1;
    }
    let open_decisions: Vec<DecisionId> = l
        .state
        .open_decisions()
        .filter(|d| d.class == DecisionClass::Substantive && scope.decision(l, d))
        .map(|d| d.id)
        .collect();
    let faults: Vec<AgentName> = l
        .state
        .faults
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(seat, _)| seat.clone())
        .collect();
    let ready_to_close = items > 0 && merged == items;

    let mut parts = Vec::new();
    if items == 0 {
        parts.push("no items yet: add one with `ris item add \"<goal>\"`".to_owned());
    } else if ready_to_close {
        parts.push(format!(
            "all {items} items merged: ready to close with `ris phase close`"
        ));
    } else {
        parts.push(format!("{merged} of {items} items merged"));
        if !in_progress.is_empty() {
            parts.push(format!(
                "{} in progress ({})",
                in_progress.len(),
                in_progress
                    .iter()
                    .map(|p| match &p.seat {
                        Some(seat) => format!("{seat} on {}", p.item),
                        None => p.item.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !set_aside.is_empty() {
            parts.push(format!("{} set aside", set_aside.len()));
        }
        parts.push(format!("{not_started} not started"));
    }
    if !open_decisions.is_empty() {
        parts.push(format!(
            "{} open decision(s) wait on you: `ris decide`",
            open_decisions.len()
        ));
    }
    for seat in &faults {
        parts.push(format!(
            "a setup fault on {seat} waits on you: `ris status` says what, then `ris up {seat}`"
        ));
    }
    if items > 0 && !ready_to_close && running.is_empty() {
        parts.push("no seat is running: start them with `ris up`".to_owned());
    }
    Some(schema::PhaseState {
        phase,
        line: parts.join("; "),
        items,
        merged,
        in_progress,
        set_aside,
        not_started,
        open_decisions,
        faults,
        running: running.to_vec(),
        ready_to_close,
    })
}

/// `ris summary`: the review view, one picture of the open phase. Its
/// goal, its items counted by state, and the graph: the phase's items with their
/// states, what they wait on, and the open decisions attached to any of
/// those. What is left out is counted in a footer that names the
/// command showing it. With no phase open it says so and how to open
/// one.
///
/// `all` is the long view of every phase instead: the decision queue,
/// the graph of all work in motion, the report and what is ready, each
/// section the same as its own command.
///
/// The JSON carries the same keys either way; by default `inbox`,
/// `report` and `ready` cover the open phase alone. `checks` is what
/// running the person's action checks did before this.
pub fn summary(
    l: &Loaded,
    actor: &Actor,
    all: bool,
    checks: Vec<String>,
    running: &[AgentName],
) -> Output {
    use crate::graph::Node;
    let scope = Scope::of(l, all);
    let state = phase_state(l, running);
    let (queue_text, queue) = inbox_in(l, actor, scope);
    let (rep_text, rep) = report_in(l, scope);
    let (rdy_text, rdy) = ready_in(l, scope);
    let graph = crate::graph::Graph::of_scope(l, scope);
    // Unsettled work left out: items not merged or superseded and open
    // decisions that are outside the scope and not drawn in the graph.
    let carried_items = l
        .state
        .items
        .values()
        .filter(|i| {
            !scope.item(i)
                && !graph.nodes.contains(&Node::Item(i.id))
                && !matches!(
                    i.state,
                    ItemState::Merged { .. } | ItemState::Superseded { .. }
                )
        })
        .count();
    let carried_decisions = l
        .state
        .open_decisions()
        .filter(|d| !scope.decision(l, d) && !graph.nodes.contains(&Node::Decision(d.id)))
        .count();
    let carried = if carried_items + carried_decisions > 0 {
        format!(
            "{carried_items} item(s) and {carried_decisions} open decision(s) from earlier phases are not shown\n  \
             next: `ris summary --all` shows them; the person brings one in with `ris item bring-in <item>`\n"
        )
    } else {
        String::new()
    };
    let text = if all {
        format!(
            "{}\n{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}",
            style::header("== decisions =="),
            queue_text,
            style::header("== what waits on what =="),
            graph.draw(l),
            style::header("== report =="),
            rep_text,
            style::header("== ready =="),
            rdy_text
        )
    } else if let Some(phase) = l.state.open_phase() {
        let mut counts = std::collections::BTreeMap::new();
        for i in l.state.items.values().filter(|i| scope.item(i)) {
            *counts.entry(shown_state(l, i)).or_insert(0usize) += 1;
        }
        let mut s = String::new();
        if let Some(state) = &state {
            s.push_str(&format!("{}\n", state.line));
        }
        s.push_str(&format!(
            "{} open: {}\n",
            phase.id,
            l.text(phase.at, "goal")
        ));
        if !counts.is_empty() {
            s.push_str(&format!(
                "items: {}\n",
                counts
                    .iter()
                    .map(|(k, v)| format!("{v} {k}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        s.push_str(&format!("\n{}\n", graph.draw(l)));
        let open = l
            .state
            .open_decisions()
            .filter(|d| d.class == DecisionClass::Substantive)
            .count();
        let mut foot = String::from("--\n");
        if open > 0 {
            foot.push_str(&format!(
                "{open} open decision(s) in all, for the person\n  next: `ris decide` walks them; answer each with one of its options or with your own\n"
            ));
        }
        foot.push_str(&carried);
        foot.push_str("the report, decisions taken and failed checks: `ris report`");
        s.push_str(&style::dim(&foot));
        s
    } else {
        format!("no phase is open\n  next: open one with `ris phase open \"<goal>\"`\n{carried}")
            .trim_end()
            .to_owned()
    };
    Output::of(
        with_checks(text, &checks),
        &schema::Summary {
            inbox: queue,
            graph: graph.json(l),
            report: rep,
            ready: rdy,
            all,
            phase_state: state,
            carried_over: schema::CarriedOver {
                items: carried_items,
                decisions: carried_decisions,
            },
            checks,
        },
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
    Output::of(
        text,
        &schema::Audit::Queue(schema::AuditQueue {
            queue: queue.iter().map(|i| item_json(l, i)).collect(),
        }),
    )
}
