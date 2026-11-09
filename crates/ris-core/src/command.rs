//! Commands: what an actor asks the harness to do, and `apply`, which
//! turns a command and the current state into events or a refusal.
//!
//! `apply` is pure. It yields *effects*, each a structural part and a
//! text part; the shell seals them into events with the salts it
//! supplies ([`seal`]), appends them, and folds them into its state.

use crate::event::{Event, Salt, Structure, TextPart};
use crate::ids::{Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, Hash, ItemId};
use crate::primitives::{Blocker, DecisionClass, DecisionState, ItemState, MessageKind};
use crate::refusal::Refusal;
use crate::state::State;
use crate::text::Text;
use crate::transitions::{self, CommandKind, DecisionStateKind, ItemStateKind, Move};

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
        chosen: usize,
        rationale: Text,
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
    MessageSend {
        from: Actor,
        to: AgentName,
        kind: MessageKind,
        about: Option<ItemId>,
        body: Text,
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
            Self::CheckRan { .. } => CommandKind::CheckRan,
            Self::Done { .. } => CommandKind::Done,
            Self::Merge { .. } => CommandKind::Merge,
            Self::MessageSend { .. } => CommandKind::MessageSend,
        }
    }
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
    }
}

fn item_state_name(k: ItemStateKind) -> &'static str {
    match k {
        ItemStateKind::Ready => "ready",
        ItemStateKind::Claimed => "claimed",
        ItemStateKind::Blocked => "blocked",
        ItemStateKind::Done => "done",
        ItemStateKind::Merged => "merged",
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
            item_move(state, *item, kind)?;
            let i = &state.items[item];
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
            if *chosen >= d.options {
                return Err(Refusal::NoSuchOption {
                    decision: *decision,
                    chosen: *chosen,
                    options: d.options,
                });
            }
            let mut effects = vec![
                Effect::new(Structure::DecisionTaken {
                    decision: *decision,
                    by: by.clone(),
                    chosen: *chosen,
                })
                .with("rationale", rationale),
            ];
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

/// Seal effects into events that follow `state`'s head, one salt each.
pub fn seal(state: &State, effects: Vec<Effect>, mut salt: impl FnMut() -> Salt) -> Vec<Event> {
    let mut prev = state.head;
    let mut seq = state.len;
    effects
        .into_iter()
        .map(|e| {
            seq += 1;
            let event = Event::new(seq, prev, e.structure, e.text, salt());
            prev = event.hash;
            event
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::PhaseId;
    use crate::primitives::PhaseState;
    use crate::text::Policy;
    use crate::transitions::PhaseStateKind;

    fn t(s: &str) -> Text {
        Text::new("test", s, &Policy::default()).unwrap()
    }

    fn agent(name: &str) -> AgentName {
        AgentName(name.into())
    }

    fn evidence(n: u8) -> EvidenceId {
        EvidenceId(Hash([n; 32]))
    }

    /// A deterministic shell: applies, seals and folds, keeping the log.
    struct Harness {
        state: State,
        log: Vec<Event>,
        n: u8,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                state: State::default(),
                log: Vec::new(),
                n: 0,
            }
        }

        fn run(&mut self, c: Command) -> Result<(), Refusal> {
            let effects = apply(&self.state, &c)?;
            let events = seal(&self.state, effects, || {
                self.n = self.n.wrapping_add(1);
                Salt([self.n; 16])
            });
            for e in &events {
                self.state.apply_event(e).unwrap();
            }
            self.log.extend(events);
            Ok(())
        }

        fn must(&mut self, c: Command) {
            let kind = c.kind();
            self.run(c)
                .unwrap_or_else(|r| panic!("{kind:?} refused: {r}"));
        }
    }

    fn with_phase_and_item() -> Harness {
        let mut h = Harness::new();
        h.must(Command::PhaseOpen { goal: t("slice") });
        h.must(Command::ItemAdd {
            goal: t("an item"),
            checks: vec![CheckId("test".into())],
            depends_on: vec![],
        });
        h
    }

    /// Drive an item into each state kind, for the table test.
    fn item_in(kind: ItemStateKind) -> Harness {
        let mut h = with_phase_and_item();
        let item = ItemId(1);
        if kind == ItemStateKind::Ready {
            return h;
        }
        h.must(Command::Claim {
            item,
            agent: agent("heron"),
        });
        match kind {
            ItemStateKind::Claimed => {}
            ItemStateKind::Blocked => h.must(Command::DecisionRaise {
                by: Actor::Agent(agent("heron")),
                about: Some(item),
                class: DecisionClass::Substantive,
                question: t("which?"),
                options: vec![t("a"), t("b")],
                recommended: 0,
                rationale: t(""),
            }),
            ItemStateKind::Done | ItemStateKind::Merged => {
                h.must(Command::Done {
                    item,
                    by: agent("heron"),
                    evidence: vec![evidence(1)],
                });
                if kind == ItemStateKind::Merged {
                    h.must(Command::Merge {
                        item,
                        commit: CommitHash("abc".into()),
                    });
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(item_state_kind(&h.state.items[&item].state), kind);
        h
    }

    /// A well-formed command of each kind that acts on item I-1 as
    /// agent heron, for the table test.
    fn item_command(kind: CommandKind) -> Option<Command> {
        let item = ItemId(1);
        Some(match kind {
            CommandKind::Claim => Command::Claim {
                item,
                agent: agent("heron"),
            },
            CommandKind::RaiseMechanical => Command::DecisionRaise {
                by: Actor::Agent(agent("heron")),
                about: Some(item),
                class: DecisionClass::Mechanical,
                question: t("q"),
                options: vec![t("a"), t("b")],
                recommended: 1,
                rationale: t("because"),
            },
            CommandKind::RaiseSubstantive => Command::DecisionRaise {
                by: Actor::Agent(agent("heron")),
                about: Some(item),
                class: DecisionClass::Substantive,
                question: t("q"),
                options: vec![t("a"), t("b")],
                recommended: 0,
                rationale: t(""),
            },
            CommandKind::CheckRan => Command::CheckRan {
                item,
                check: CheckId("test".into()),
                passed: true,
                evidence: evidence(2),
            },
            CommandKind::Done => Command::Done {
                item,
                by: agent("heron"),
                evidence: vec![evidence(3)],
            },
            CommandKind::Merge => Command::Merge {
                item,
                commit: CommitHash("def".into()),
            },
            // Decide acts on a decision, and reaches the item through
            // the blocker; covered by the decision table and a test
            // below.
            CommandKind::Decide
            | CommandKind::PhaseOpen
            | CommandKind::PhaseClose
            | CommandKind::ItemAdd
            | CommandKind::MessageSend => return None,
        })
    }

    #[test]
    fn apply_follows_the_item_table() {
        for &state_kind in ItemStateKind::ALL {
            for &command_kind in CommandKind::ALL {
                let Some(command) = item_command(command_kind) else {
                    continue;
                };
                let mut h = item_in(state_kind);
                let expected = transitions::lookup(transitions::ITEM, state_kind, command_kind);
                let result = h.run(command);
                match expected {
                    None => assert!(
                        matches!(result, Err(Refusal::IllegalItemTransition { .. })),
                        "{state_kind:?} + {command_kind:?}: expected a refusal, got {result:?}"
                    ),
                    Some(Move::Stay) => {
                        result.unwrap();
                        assert_eq!(
                            item_state_kind(&h.state.items[&ItemId(1)].state),
                            state_kind
                        );
                    }
                    Some(Move::To(next)) => {
                        result.unwrap();
                        assert_eq!(item_state_kind(&h.state.items[&ItemId(1)].state), next);
                    }
                }
            }
        }
    }

    #[test]
    fn decide_unblocks_the_item_and_follows_the_decision_table() {
        let mut h = item_in(ItemStateKind::Blocked);
        let decide = |by: Actor| Command::Decide {
            decision: DecisionId(1),
            by,
            chosen: 1,
            rationale: t("b is cheaper to reverse"),
        };
        // Substantive: the person's to take.
        assert_eq!(
            h.run(decide(Actor::Agent(agent("heron")))),
            Err(Refusal::SubstantiveIsForThePerson(DecisionId(1)))
        );
        h.must(decide(Actor::Person));
        assert_eq!(h.state.items[&ItemId(1)].state, ItemState::Ready);
        assert!(matches!(
            h.state.decisions[&DecisionId(1)].state,
            DecisionState::Taken { chosen: 1, .. }
        ));
        // Taken is terminal.
        assert!(matches!(
            h.run(decide(Actor::Person)),
            Err(Refusal::IllegalDecisionTransition { state: "taken", .. })
        ));
        assert_eq!(
            transitions::lookup(
                transitions::DECISION,
                DecisionStateKind::Taken,
                CommandKind::Decide
            ),
            None
        );
    }

    #[test]
    fn a_mechanical_decision_is_taken_at_once_by_its_raiser() {
        let mut h = item_in(ItemStateKind::Claimed);
        h.must(item_command(CommandKind::RaiseMechanical).unwrap());
        let d = &h.state.decisions[&DecisionId(1)];
        assert!(matches!(
            d.state,
            DecisionState::Taken {
                by: Actor::Agent(_),
                chosen: 1,
                ..
            }
        ));
        assert_eq!(h.log.len(), 5, "raised and taken are two events");
        assert_eq!(
            h.log[4].text_field("rationale").unwrap().as_str(),
            "because"
        );
    }

    #[test]
    fn a_second_claim_is_refused_and_only_the_holder_finishes() {
        let mut h = item_in(ItemStateKind::Claimed);
        assert!(matches!(
            h.run(Command::Claim {
                item: ItemId(1),
                agent: agent("wren")
            }),
            Err(Refusal::IllegalItemTransition {
                state: "claimed",
                command: CommandKind::Claim,
                ..
            })
        ));
        assert_eq!(
            h.run(Command::Done {
                item: ItemId(1),
                by: agent("wren"),
                evidence: vec![evidence(1)],
            }),
            Err(Refusal::NotTheClaimant {
                item: ItemId(1),
                holder: agent("heron")
            })
        );
        assert_eq!(
            h.run(Command::Done {
                item: ItemId(1),
                by: agent("heron"),
                evidence: vec![],
            }),
            Err(Refusal::NoEvidence(ItemId(1)))
        );
    }

    #[test]
    fn dependencies_must_land_before_a_claim() {
        let mut h = with_phase_and_item();
        h.must(Command::ItemAdd {
            goal: t("second"),
            checks: vec![],
            depends_on: vec![ItemId(1)],
        });
        assert_eq!(h.state.ready().count(), 1);
        assert_eq!(
            h.run(Command::Claim {
                item: ItemId(2),
                agent: agent("wren")
            }),
            Err(Refusal::DependenciesOpen {
                item: ItemId(2),
                waiting_on: vec![ItemId(1)]
            })
        );
        h.must(Command::Claim {
            item: ItemId(1),
            agent: agent("heron"),
        });
        h.must(Command::Done {
            item: ItemId(1),
            by: agent("heron"),
            evidence: vec![evidence(1)],
        });
        h.must(Command::Merge {
            item: ItemId(1),
            commit: CommitHash("abc".into()),
        });
        h.must(Command::Claim {
            item: ItemId(2),
            agent: agent("wren"),
        });
    }

    #[test]
    fn phases_follow_their_table() {
        let mut h = Harness::new();
        assert_eq!(h.run(Command::PhaseClose), Err(Refusal::NoOpenPhase));
        assert_eq!(
            h.run(Command::ItemAdd {
                goal: t("x"),
                checks: vec![],
                depends_on: vec![]
            }),
            Err(Refusal::NoOpenPhase)
        );
        h.must(Command::PhaseOpen { goal: t("one") });
        assert_eq!(
            h.run(Command::PhaseOpen { goal: t("two") }),
            Err(Refusal::PhaseAlreadyOpen)
        );
        let head_before_close = h.state.head;
        h.must(Command::PhaseClose);
        let PhaseState::Closed { report } = h.state.phases[&PhaseId(1)].state else {
            panic!("phase should be closed");
        };
        assert_eq!(report, report_hash(&Hash::ZERO, &head_before_close));
        assert_eq!(h.state.last_report, report);
        h.must(Command::PhaseOpen { goal: t("two") });
        assert_eq!(h.state.open_phase().unwrap().id, PhaseId(2));
        assert!(
            transitions::lookup(
                transitions::PHASE,
                PhaseStateKind::Open,
                CommandKind::PhaseOpen
            )
            .is_none()
        );
    }

    #[test]
    fn messages_go_to_another_agent() {
        let mut h = with_phase_and_item();
        let send = |from: Actor, to: &str| Command::MessageSend {
            from,
            to: agent(to),
            kind: MessageKind::Question,
            about: Some(ItemId(1)),
            body: t("is the interface settled?"),
        };
        assert_eq!(
            h.run(send(Actor::Agent(agent("heron")), "heron")),
            Err(Refusal::MessageToSelf)
        );
        h.must(send(Actor::Agent(agent("wren")), "heron"));
        h.must(send(Actor::Person, "heron"));
        assert_eq!(h.state.messages.len(), 2);
    }

    #[test]
    fn fold_of_the_log_equals_the_live_state_and_ignores_text() {
        let mut h = item_in(ItemStateKind::Merged);
        h.must(Command::PhaseClose);
        let replayed = State::fold(&h.log).unwrap();
        assert_eq!(replayed, h.state);
        let stripped: Vec<Event> = h.log.iter().cloned().map(Event::redacted).collect();
        assert_eq!(State::fold(&stripped).unwrap(), h.state);
        assert!(crate::event::verify(&h.log).is_ok());
        assert_eq!(
            crate::event::verify(&stripped).unwrap().missing_text.len(),
            h.log.len()
        );
    }
}
