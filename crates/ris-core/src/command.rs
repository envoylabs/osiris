//! Commands: what an actor asks the harness to do, and `apply`, which
//! turns a command and the current state into events or a refusal.
//!
//! `apply` is pure. It yields *effects*, each a structural part and a
//! text part; the shell seals them into events with the salts it
//! supplies ([`seal`]), appends them, and folds them into its state.

use crate::event::{Event, Salt, Structure, TextPart};
use crate::ids::{
    ActionId, Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, Hash, ItemId,
};
use crate::primitives::ActionState;
use crate::primitives::{Blocker, DecisionClass, DecisionState, ItemState, MessageKind};
use crate::refusal::Refusal;
use crate::state::State;
use crate::text::Text;
use crate::transitions::{
    self, ActionStateKind, CommandKind, DecisionStateKind, ItemStateKind, Move,
};

/// What an actor asks for. Every free-text field is a [`Text`], so it
/// has already passed the confidentiality scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PhaseOpen {
        goal: Text,
    },
    /// Closes the open phase. The report hash is taken over the log's
    /// head and the previous report, never over a rendering.
    PhaseClose,
    ItemAdd {
        goal: Text,
        checks: Vec<CheckId>,
        depends_on: Vec<ItemId>,
    },
    Claim {
        item: ItemId,
        agent: AgentName,
    },
    /// Raise a decision. A mechanical one is taken at once by the
    /// raiser, with its recommendation, and logged; a substantive one is
    /// queued for the person and sets the item it is about aside.
    DecisionRaise {
        by: Actor,
        about: Option<ItemId>,
        class: DecisionClass,
        question: Text,
        options: Vec<Text>,
        recommended: usize,
        /// For a mechanical decision, why the recommendation holds.
        rationale: Text,
    },
    Decide {
        decision: DecisionId,
        by: Actor,
        chosen: Choice,
        rationale: Text,
    },
    /// The full case for an open decision, for the person to read
    /// before deciding: what it is about, pros and cons per option as
    /// `(option, text)`, and why the recommendation holds.
    Explain {
        decision: DecisionId,
        by: Actor,
        summary: Text,
        pros: Vec<(usize, Text)>,
        cons: Vec<(usize, Text)>,
        why: Text,
    },
    /// Record a check's run against an item. Any state but Merged.
    CheckRan {
        item: ItemId,
        check: CheckId,
        passed: bool,
        evidence: EvidenceId,
    },
    Done {
        item: ItemId,
        by: AgentName,
        evidence: Vec<EvidenceId>,
    },
    Merge {
        item: ItemId,
        commit: CommitHash,
    },
    /// Record that an auditor has reviewed a done or merged item.
    Audit {
        item: ItemId,
        by: AgentName,
    },
    /// Return a claimed item to ready. The holder may; so may the
    /// person, for a claim whose agent has gone.
    Release {
        item: ItemId,
        by: Actor,
    },
    MessageSend {
        from: Actor,
        to: AgentName,
        kind: MessageKind,
        about: Option<ItemId>,
        body: Text,
    },
    /// Raise a person action: what to do and how to tell it is done,
    /// the items that wait for it (`blocks`) and those it shapes
    /// without holding them up (`informs`). The person's only: an
    /// action is their own to-do. An agent that is blocked raises an
    /// item or a decision instead.
    RaiseAction {
        by: Actor,
        what: Text,
        done_when: Text,
        blocks: Vec<ItemId>,
        informs: Vec<ItemId>,
    },
    /// Give an open action a check: a shell command that passes once it
    /// is done. The person's only, since the person's own commands run
    /// it, outside any agent's sandbox.
    SetActionCheck {
        action: ActionId,
        by: Actor,
        check: Text,
    },
    /// Record a person action done, with evidence or a note. The
    /// person's only.
    ActionDone {
        action: ActionId,
        by: Actor,
        evidence: Vec<EvidenceId>,
        note: Option<Text>,
    },
    /// The runner started a seat's agent, on the person's command. A
    /// paused seat, or one already running, is refused.
    RunStart {
        seat: AgentName,
        actor: Actor,
        runner: Text,
        model: Option<Text>,
    },
    /// The seat's agent stopped.
    RunStop {
        seat: AgentName,
        actor: Actor,
        status: Option<i32>,
    },
    /// A seat reports a fault in its setup (a path it cannot write, a
    /// host it cannot reach, a command it may not run). Not a decision:
    /// nothing is chosen; the person fixes the configuration. Any
    /// seat's, about itself only.
    SetupFault {
        seat: AgentName,
        what: Text,
    },
    /// Pause seats: each finishes what it holds, and its next claim or
    /// audit is refused until it is resumed. The person's only.
    Pause {
        seats: Vec<AgentName>,
        actor: Actor,
    },
    Resume {
        seats: Vec<AgentName>,
        actor: Actor,
    },
    /// Replace an item that has not been started with another, which
    /// must exist and stand for itself. The person's only.
    Supersede {
        item: ItemId,
        by: ItemId,
        actor: Actor,
    },
    /// Bring an unstarted item from the backlog into the open phase.
    /// The person's only.
    BringIn {
        item: ItemId,
        actor: Actor,
    },
}

impl Command {
    /// The kind the transition table speaks of.
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::PhaseOpen { .. } => CommandKind::PhaseOpen,
            Self::PhaseClose => CommandKind::PhaseClose,
            Self::ItemAdd { .. } => CommandKind::ItemAdd,
            Self::Claim { .. } => CommandKind::Claim,
            Self::DecisionRaise {
                class: DecisionClass::Mechanical,
                ..
            } => CommandKind::RaiseMechanical,
            Self::DecisionRaise {
                class: DecisionClass::Substantive,
                ..
            } => CommandKind::RaiseSubstantive,
            Self::Decide { .. } => CommandKind::Decide,
            Self::Explain { .. } => CommandKind::Explain,
            Self::CheckRan { .. } => CommandKind::CheckRan,
            Self::Done { .. } => CommandKind::Done,
            Self::Merge { .. } => CommandKind::Merge,
            Self::MessageSend { .. } => CommandKind::MessageSend,
            Self::Release { .. } => CommandKind::Release,
            Self::Audit { .. } => CommandKind::Audit,
            Self::Supersede { .. } => CommandKind::Supersede,
            Self::BringIn { .. } => CommandKind::BringIn,
            Self::RunStart { .. } => CommandKind::RunStart,
            Self::RunStop { .. } => CommandKind::RunStop,
            Self::SetupFault { .. } => CommandKind::SetupFault,
            Self::RaiseAction { .. } => CommandKind::RaiseAction,
            Self::ActionDone { .. } => CommandKind::ActionDone,
            Self::SetActionCheck { .. } => CommandKind::SetActionCheck,
            Self::Pause { .. } => CommandKind::Pause,
            Self::Resume { .. } => CommandKind::Resume,
        }
    }
}

/// What a decider picks: one of the offered options, or an answer of
/// their own. An answer outside the options is recorded as one more
/// option on the decision (index `options`), so the record is complete
/// and the report can mark it. How often that happens per phase says
/// how well the agents are framing options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Option(usize),
    Other(Text),
}

/// One event's worth of outcome, before it is sealed into the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub structure: Structure,
    pub text: TextPart,
}

impl Effect {
    fn new(structure: Structure) -> Self {
        Self {
            structure,
            text: TextPart::new(),
        }
    }

    fn with(mut self, field: &str, text: &Text) -> Self {
        self.text.insert(field.to_owned(), text.clone());
        self
    }
}

fn item_state_kind(s: &ItemState) -> ItemStateKind {
    match s {
        ItemState::Ready => ItemStateKind::Ready,
        ItemState::Claimed { .. } => ItemStateKind::Claimed,
        ItemState::Blocked { .. } => ItemStateKind::Blocked,
        ItemState::Done { .. } => ItemStateKind::Done,
        ItemState::Merged { .. } => ItemStateKind::Merged,
        ItemState::Superseded { .. } => ItemStateKind::Superseded,
    }
}

fn item_state_name(k: ItemStateKind) -> &'static str {
    match k {
        ItemStateKind::Ready => "ready",
        ItemStateKind::Claimed => "claimed",
        ItemStateKind::Blocked => "blocked",
        ItemStateKind::Done => "done",
        ItemStateKind::Merged => "merged",
        ItemStateKind::Superseded => "superseded",
    }
}

fn decision_state_kind(s: &DecisionState) -> DecisionStateKind {
    match s {
        DecisionState::Open => DecisionStateKind::Open,
        DecisionState::Taken { .. } => DecisionStateKind::Taken,
        DecisionState::Superseded(_) => DecisionStateKind::Superseded,
    }
}

fn decision_state_name(k: DecisionStateKind) -> &'static str {
    match k {
        DecisionStateKind::Open => "open",
        DecisionStateKind::Taken => "taken",
        DecisionStateKind::Superseded => "superseded",
    }
}

/// Look the item's move up in the table, or refuse.
fn item_move(
    state: &State,
    item: ItemId,
    command: CommandKind,
) -> Result<Move<ItemStateKind>, Refusal> {
    let i = state.items.get(&item).ok_or(Refusal::UnknownItem(item))?;
    let kind = item_state_kind(&i.state);
    transitions::lookup(transitions::ITEM, kind, command).ok_or(Refusal::IllegalItemTransition {
        item,
        state: item_state_name(kind),
        command,
    })
}

fn decision_move(
    state: &State,
    decision: DecisionId,
    command: CommandKind,
) -> Result<Move<DecisionStateKind>, Refusal> {
    let d = state
        .decisions
        .get(&decision)
        .ok_or(Refusal::UnknownDecision(decision))?;
    let kind = decision_state_kind(&d.state);
    transitions::lookup(transitions::DECISION, kind, command).ok_or(
        Refusal::IllegalDecisionTransition {
            decision,
            state: decision_state_name(kind),
            command,
        },
    )
}

/// Look the action's move up in the table, or refuse.
fn action_move(
    state: &State,
    action: ActionId,
    command: CommandKind,
) -> Result<Move<ActionStateKind>, Refusal> {
    let a = state
        .actions
        .get(&action)
        .ok_or(Refusal::UnknownAction(action))?;
    let k = match a.state {
        ActionState::Open => ActionStateKind::Open,
        ActionState::Done { .. } => ActionStateKind::Done,
    };
    transitions::lookup(transitions::ACTION, k, command).ok_or(Refusal::IllegalActionTransition {
        action,
        state: if k == ActionStateKind::Open {
            "open"
        } else {
            "done"
        },
        command,
    })
}

/// `H("ris-report-v1" || previous report || log head)`.
pub fn report_hash(previous: &Hash, head: &Hash) -> Hash {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"ris-report-v1");
    h.update(previous.0);
    h.update(head.0);
    Hash(h.finalize().into())
}

/// Apply a command to the state: the effects to append, or a refusal.
/// Nothing is appended here; the shell seals and stores the effects.
pub fn apply(state: &State, command: &Command) -> Result<Vec<Effect>, Refusal> {
    let kind = command.kind();
    match command {
        Command::PhaseOpen { goal } => {
            if state.open_phase().is_some() {
                return Err(Refusal::PhaseAlreadyOpen);
            }
            let phase = state.next_phase_id();
            Ok(vec![
                Effect::new(Structure::PhaseOpened { phase }).with("goal", goal),
            ])
        }

        Command::PhaseClose => {
            let phase = state.open_phase().ok_or(Refusal::NoOpenPhase)?.id;
            // The report covers the log up to and including this close;
            // the close's own hash is not known until sealed, so the
            // report is taken over the head before it. Verification
            // recomputes it the same way.
            let report = report_hash(&state.last_report, &state.head);
            Ok(vec![Effect::new(Structure::PhaseClosed { phase, report })])
        }

        Command::ItemAdd {
            goal,
            checks,
            depends_on,
        } => {
            let phase = state.open_phase().ok_or(Refusal::NoOpenPhase)?.id;
            for d in depends_on {
                if !state.items.contains_key(d) {
                    return Err(Refusal::UnknownItem(*d));
                }
            }
            let item = state.next_item_id();
            Ok(vec![
                Effect::new(Structure::ItemAdded {
                    item,
                    phase,
                    checks: checks.clone(),
                    depends_on: depends_on.clone(),
                })
                .with("goal", goal),
            ])
        }

        Command::Claim { item, agent } => {
            if state.paused.contains(agent) {
                return Err(Refusal::SeatPaused(agent.clone()));
            }
            item_move(state, *item, kind)?;
            let i = &state.items[item];
            // An agent works the open phase's items only; the person
            // brings a backlog item in first.
            let open = state.open_phase().ok_or(Refusal::NoOpenPhase)?.id;
            if open != i.phase {
                return Err(Refusal::NotInOpenPhase {
                    item: *item,
                    phase: i.phase,
                });
            }
            let waiting_on = state.unlanded_dependencies(i);
            if !waiting_on.is_empty() {
                return Err(Refusal::DependenciesOpen {
                    item: *item,
                    waiting_on,
                });
            }
            Ok(vec![Effect::new(Structure::ItemClaimed {
                item: *item,
                agent: agent.clone(),
            })])
        }

        Command::DecisionRaise {
            by,
            about,
            class,
            question,
            options,
            recommended,
            rationale,
        } => {
            if options.len() < 2 || *recommended >= options.len() {
                return Err(Refusal::MalformedDecision);
            }
            if let Some(item) = about {
                item_move(state, *item, kind)?;
                require_claimant(state, *item, by)?;
            }
            let decision = state.next_decision_id();
            let mut raised = Effect::new(Structure::DecisionRaised {
                decision,
                class: *class,
                by: by.clone(),
                about: *about,
                options: options.len(),
                recommended: *recommended,
            })
            .with("question", question);
            for (n, o) in options.iter().enumerate() {
                raised = raised.with(&format!("option.{n}"), o);
            }
            let mut effects = vec![raised];
            match class {
                DecisionClass::Mechanical => effects.push(
                    Effect::new(Structure::DecisionTaken {
                        decision,
                        by: by.clone(),
                        chosen: *recommended,
                    })
                    .with("rationale", rationale),
                ),
                DecisionClass::Substantive => {
                    if let Some(item) = about {
                        effects.push(Effect::new(Structure::ItemBlocked {
                            item: *item,
                            on: vec![Blocker::Decision(decision)],
                        }));
                    }
                }
            }
            Ok(effects)
        }

        Command::Explain {
            decision,
            by,
            summary,
            pros,
            cons,
            why,
        } => {
            decision_move(state, *decision, kind)?;
            let d = &state.decisions[decision];
            if *by != Actor::Person && *by != d.by {
                return Err(Refusal::NotTheRaiser(*decision));
            }
            for (n, _) in pros.iter().chain(cons) {
                if *n >= d.options {
                    return Err(Refusal::NoSuchOption {
                        decision: *decision,
                        chosen: *n,
                        options: d.options,
                    });
                }
            }
            let count =
                |list: &[(usize, Text)], n: usize| list.iter().filter(|(o, _)| *o == n).count();
            let pro_counts: Vec<usize> = (0..d.options).map(|n| count(pros, n)).collect();
            let con_counts: Vec<usize> = (0..d.options).map(|n| count(cons, n)).collect();
            let thin = summary.as_str().trim().is_empty()
                || why.as_str().trim().is_empty()
                || (0..d.options).any(|n| pro_counts[n] + con_counts[n] == 0);
            if thin {
                return Err(Refusal::ThinExplanation(*decision));
            }
            let mut effect = Effect::new(Structure::DecisionExplained {
                decision: *decision,
                by: by.clone(),
                pros: pro_counts,
                cons: con_counts,
            })
            .with("summary", summary)
            .with("why", why);
            for (label, list) in [("pro", pros), ("con", cons)] {
                for n in 0..d.options {
                    for (k, (_, text)) in list.iter().filter(|(o, _)| *o == n).enumerate() {
                        effect = effect.with(&format!("option.{n}.{label}.{k}"), text);
                    }
                }
            }
            Ok(vec![effect])
        }

        Command::Decide {
            decision,
            by,
            chosen,
            rationale,
        } => {
            decision_move(state, *decision, kind)?;
            let d = &state.decisions[decision];
            if d.class == DecisionClass::Substantive && *by != Actor::Person {
                return Err(Refusal::SubstantiveIsForThePerson(*decision));
            }
            let taken = match chosen {
                Choice::Option(n) => {
                    if *n >= d.options {
                        return Err(Refusal::NoSuchOption {
                            decision: *decision,
                            chosen: *n,
                            options: d.options,
                        });
                    }
                    Effect::new(Structure::DecisionTaken {
                        decision: *decision,
                        by: by.clone(),
                        chosen: *n,
                    })
                }
                // One past the offered options; the answer's text rides on
                // this event as `other`.
                Choice::Other(answer) => Effect::new(Structure::DecisionTaken {
                    decision: *decision,
                    by: by.clone(),
                    chosen: d.options,
                })
                .with("other", answer),
            };
            let mut effects = vec![taken.with("rationale", rationale)];
            // Items blocked only on this decision come back to Ready.
            for i in state.items.values() {
                if let ItemState::Blocked { on } = &i.state
                    && on.iter().all(|b| *b == Blocker::Decision(*decision))
                {
                    effects.push(Effect::new(Structure::ItemReady { item: i.id }));
                }
            }
            Ok(effects)
        }

        Command::CheckRan {
            item,
            check,
            passed,
            evidence,
        } => {
            item_move(state, *item, kind)?;
            Ok(vec![Effect::new(Structure::CheckRan {
                item: *item,
                check: check.clone(),
                passed: *passed,
                evidence: *evidence,
            })])
        }

        Command::Done { item, by, evidence } => {
            item_move(state, *item, kind)?;
            require_claimant(state, *item, &Actor::Agent(by.clone()))?;
            if evidence.is_empty() {
                return Err(Refusal::NoEvidence(*item));
            }
            Ok(vec![Effect::new(Structure::ItemDone {
                item: *item,
                evidence: evidence.clone(),
            })])
        }

        Command::Merge { item, commit } => {
            item_move(state, *item, kind)?;
            Ok(vec![Effect::new(Structure::ItemMerged {
                item: *item,
                commit: commit.clone(),
            })])
        }

        Command::Audit { item, by } => {
            if state.paused.contains(by) {
                return Err(Refusal::SeatPaused(by.clone()));
            }
            item_move(state, *item, kind)?;
            let i = &state.items[item];
            if i.author.as_ref() == Some(by) {
                return Err(Refusal::AuditOwnWork(*item));
            }
            if i.audited_by.contains(by) {
                return Err(Refusal::AlreadyAudited {
                    item: *item,
                    by: by.clone(),
                });
            }
            Ok(vec![Effect::new(Structure::ItemAudited {
                item: *item,
                by: by.clone(),
            })])
        }

        Command::Release { item, by } => {
            item_move(state, *item, kind)?;
            if *by != Actor::Person {
                require_claimant(state, *item, by)?;
            }
            Ok(vec![Effect::new(Structure::ItemReady { item: *item })])
        }

        Command::RunStart {
            seat,
            actor,
            runner,
            model,
        } => {
            if *actor != Actor::Person {
                return Err(Refusal::SeatControlIsForThePerson);
            }
            if state.paused.contains(seat) {
                return Err(Refusal::SeatPaused(seat.clone()));
            }
            if let Some(at) = state.running.get(seat) {
                return Err(Refusal::AlreadyRunning {
                    seat: seat.clone(),
                    since: *at,
                });
            }
            let mut e =
                Effect::new(Structure::RunStarted { seat: seat.clone() }).with("runner", runner);
            if let Some(m) = model {
                e = e.with("model", m);
            }
            Ok(vec![e])
        }

        Command::RunStop {
            seat,
            actor,
            status,
        } => {
            if *actor != Actor::Person {
                return Err(Refusal::SeatControlIsForThePerson);
            }
            if !state.running.contains_key(seat) {
                return Err(Refusal::NotRunning(seat.clone()));
            }
            Ok(vec![Effect::new(Structure::RunStopped {
                seat: seat.clone(),
                status: *status,
            })])
        }

        Command::SetupFault { seat, what } => Ok(vec![
            Effect::new(Structure::SetupFault { seat: seat.clone() }).with("what", what),
        ]),

        Command::RaiseAction {
            by,
            what,
            done_when,
            blocks,
            informs,
        } => {
            if *by != Actor::Person {
                return Err(Refusal::RaiseActionIsForThePerson);
            }
            if what.as_str().trim().is_empty() || done_when.as_str().trim().is_empty() {
                return Err(Refusal::MalformedAction);
            }
            let mut effects = Vec::new();
            for item in blocks {
                item_move(state, *item, kind)?;
            }
            for item in informs {
                let i = state.items.get(item).ok_or(Refusal::UnknownItem(*item))?;
                if matches!(
                    i.state,
                    ItemState::Merged { .. } | ItemState::Superseded { .. }
                ) {
                    return Err(Refusal::SettledItem(*item));
                }
            }
            let action = state.next_action_id();
            effects.push(
                Effect::new(Structure::ActionRaised {
                    action,
                    by: by.clone(),
                    blocks: blocks.clone(),
                    informs: informs.clone(),
                })
                .with("what", what)
                .with("done_when", done_when),
            );
            for item in blocks {
                let mut on = match &state.items[item].state {
                    ItemState::Blocked { on } => on.clone(),
                    _ => Vec::new(),
                };
                on.push(Blocker::Action(action));
                effects.push(Effect::new(Structure::ItemBlocked { item: *item, on }));
            }
            Ok(effects)
        }

        Command::SetActionCheck { action, by, check } => {
            action_move(state, *action, kind)?;
            if *by != Actor::Person {
                return Err(Refusal::ActionIsForThePerson(*action));
            }
            if check.as_str().trim().is_empty() {
                return Err(Refusal::MalformedAction);
            }
            Ok(vec![
                Effect::new(Structure::ActionCheckSet { action: *action }).with("check", check),
            ])
        }

        Command::ActionDone {
            action,
            by,
            evidence,
            note,
        } => {
            let a = state
                .actions
                .get(action)
                .ok_or(Refusal::UnknownAction(*action))?;
            let k = match a.state {
                ActionState::Open => ActionStateKind::Open,
                ActionState::Done { .. } => ActionStateKind::Done,
            };
            transitions::lookup(transitions::ACTION, k, kind).ok_or(
                Refusal::IllegalActionTransition {
                    action: *action,
                    state: if k == ActionStateKind::Open {
                        "open"
                    } else {
                        "done"
                    },
                    command: kind,
                },
            )?;
            if *by != Actor::Person {
                return Err(Refusal::ActionIsForThePerson(*action));
            }
            let note = note.as_ref().filter(|n| !n.as_str().trim().is_empty());
            if evidence.is_empty() && note.is_none() {
                return Err(Refusal::ActionNeedsResult(*action));
            }
            let mut done = Effect::new(Structure::ActionDone {
                action: *action,
                evidence: evidence.clone(),
            });
            if let Some(n) = note {
                done = done.with("note", n);
            }
            let mut effects = vec![done];
            // Items blocked only on this action come back to Ready.
            for i in state.items.values() {
                if let ItemState::Blocked { on } = &i.state
                    && on.iter().all(|b| *b == Blocker::Action(*action))
                {
                    effects.push(Effect::new(Structure::ItemReady { item: i.id }));
                }
            }
            Ok(effects)
        }

        Command::Pause { seats, actor } | Command::Resume { seats, actor } => {
            if *actor != Actor::Person {
                return Err(Refusal::SeatControlIsForThePerson);
            }
            let pause = matches!(command, Command::Pause { .. });
            Ok(seats
                .iter()
                .map(|seat| {
                    Effect::new(if pause {
                        Structure::SeatPaused { seat: seat.clone() }
                    } else {
                        Structure::SeatResumed { seat: seat.clone() }
                    })
                })
                .collect())
        }

        Command::Supersede { item, by, actor } => {
            if *actor != Actor::Person {
                return Err(Refusal::SupersedeIsForThePerson(*item));
            }
            item_move(state, *item, kind)?;
            let replacement = state.items.get(by).ok_or(Refusal::UnknownItem(*by))?;
            if by == item
                || matches!(replacement.state, ItemState::Superseded { .. })
                || state.would_wait_on_itself(*item, *by)
            {
                return Err(Refusal::NotAReplacement {
                    item: *item,
                    by: *by,
                });
            }
            Ok(vec![Effect::new(Structure::ItemSuperseded {
                item: *item,
                by: *by,
            })])
        }

        Command::BringIn { item, actor } => {
            if *actor != Actor::Person {
                return Err(Refusal::BringInIsForThePerson(*item));
            }
            let open = state.open_phase().ok_or(Refusal::NoOpenPhase)?.id;
            item_move(state, *item, kind)?;
            if state.items[item].phase == open {
                return Err(Refusal::AlreadyInOpenPhase(*item));
            }
            Ok(vec![Effect::new(Structure::ItemScoped {
                item: *item,
                phase: Some(open),
            })])
        }

        Command::MessageSend {
            from,
            to,
            kind: message_kind,
            about,
            body,
        } => {
            if *from == Actor::Agent(to.clone()) {
                return Err(Refusal::MessageToSelf);
            }
            if let Some(item) = about
                && !state.items.contains_key(item)
            {
                return Err(Refusal::UnknownItem(*item));
            }
            Ok(vec![
                Effect::new(Structure::MessageSent {
                    message: state.next_message_id(),
                    from: from.clone(),
                    to: to.clone(),
                    kind: *message_kind,
                    about: *about,
                })
                .with("body", body),
            ])
        }
    }
}

/// Only the agent holding a claimed item may act on it as its author.
fn require_claimant(state: &State, item: ItemId, actor: &Actor) -> Result<(), Refusal> {
    if let ItemState::Claimed { agent } = &state.items[&item].state
        && *actor != Actor::Agent(agent.clone())
    {
        return Err(Refusal::NotTheClaimant {
            item,
            holder: agent.clone(),
        });
    }
    Ok(())
}

/// Seal effects into events that follow `state`'s head, one salt each,
/// all stamped `at` (the shell's clock, milliseconds since the epoch).
pub fn seal(
    state: &State,
    effects: Vec<Effect>,
    at: u64,
    mut salt: impl FnMut() -> Salt,
) -> Vec<Event> {
    let mut prev = state.head;
    let mut seq = state.len;
    effects
        .into_iter()
        .map(|e| {
            seq += 1;
            let event = Event::new(seq, prev, at, e.structure, e.text, salt());
            prev = event.hash;
            event
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/command.rs"]
mod tests;
