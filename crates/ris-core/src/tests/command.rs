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
        let events = seal(&self.state, effects, 1_700_000_000_000, || {
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
    // I-2 is there to replace I-1 for the Supersede rows.
    h.must(Command::ItemAdd {
        goal: t("a replacement"),
        checks: vec![],
        depends_on: vec![],
    });
    if kind == ItemStateKind::Ready {
        return h;
    }
    if kind == ItemStateKind::Superseded {
        h.must(supersede(item, ItemId(2), Actor::Person));
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
        CommandKind::Release => Command::Release {
            item,
            by: Actor::Agent(agent("heron")),
        },
        CommandKind::Audit => Command::Audit {
            item,
            by: agent("wren"),
        },
        CommandKind::Supersede => supersede(item, ItemId(2), Actor::Person),
        CommandKind::RaiseAction => Command::RaiseAction {
            by: Actor::Person,
            what: t("add the builder seat"),
            done_when: t("ris status lists it"),
            blocks: vec![item],
            informs: vec![],
        },
        // Decide acts on a decision, and reaches the item through
        // the blocker; covered by the decision table and a test
        // below.
        CommandKind::Decide
        | CommandKind::Explain
        | CommandKind::PhaseOpen
        | CommandKind::PhaseClose
        | CommandKind::ItemAdd
        | CommandKind::Pause
        | CommandKind::Resume
        | CommandKind::ActionDone
        | CommandKind::SetActionCheck
        | CommandKind::RunStart
        | CommandKind::RunStop
        | CommandKind::SetupFault
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
        chosen: Choice::Option(1),
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
fn an_open_decision_is_explained_by_its_raiser_or_the_person() {
    // D-1 is heron's substantive decision on I-1, with two options.
    let mut h = item_in(ItemStateKind::Blocked);
    let explain =
        |by: Actor, pros: Vec<(usize, Text)>, cons: Vec<(usize, Text)>| Command::Explain {
            decision: DecisionId(1),
            by,
            summary: t("which way"),
            pros,
            cons,
            why: t("a is cheaper to reverse"),
        };
    let full = || (vec![(0, t("simple")), (0, t("fast"))], vec![(1, t("slow"))]);
    let (p, c) = full();
    assert_eq!(
        h.run(explain(Actor::Agent(agent("wren")), p, c)),
        Err(Refusal::NotTheRaiser(DecisionId(1)))
    );
    assert_eq!(
        h.run(explain(
            Actor::Agent(agent("heron")),
            vec![(0, t("simple"))],
            vec![]
        )),
        Err(Refusal::ThinExplanation(DecisionId(1))),
        "option 1 has neither a pro nor a con"
    );
    assert!(matches!(
        h.run(explain(
            Actor::Agent(agent("heron")),
            vec![(2, t("x"))],
            vec![(0, t("y")), (1, t("z"))]
        )),
        Err(Refusal::NoSuchOption { chosen: 2, .. })
    ));
    let (p, c) = full();
    h.must(explain(Actor::Agent(agent("heron")), p, c));
    let e = h.state.decisions[&DecisionId(1)].explained.clone().unwrap();
    assert_eq!((e.pros, e.cons), (vec![2, 0], vec![0, 1]));
    let (p, c) = full();
    h.must(explain(Actor::Person, p, c));
    // Explaining changes nothing about the decision or its item.
    assert_eq!(h.state.decisions[&DecisionId(1)].state, DecisionState::Open);
    assert!(matches!(
        h.state.items[&ItemId(1)].state,
        ItemState::Blocked { .. }
    ));
    // Once taken, it is history: no more explaining.
    h.must(Command::Decide {
        decision: DecisionId(1),
        by: Actor::Person,
        chosen: Choice::Option(0),
        rationale: t("ok"),
    });
    let (p, c) = full();
    assert!(matches!(
        h.run(explain(Actor::Person, p, c)),
        Err(Refusal::IllegalDecisionTransition { state: "taken", .. })
    ));
}

#[test]
fn an_answer_outside_the_options_is_recorded_as_one_more() {
    let mut h = item_in(ItemStateKind::Blocked);
    assert_eq!(
        h.run(Command::Decide {
            decision: DecisionId(1),
            by: Actor::Person,
            chosen: Choice::Option(2),
            rationale: t("x"),
        }),
        Err(Refusal::NoSuchOption {
            decision: DecisionId(1),
            chosen: 2,
            options: 2
        })
    );
    h.must(Command::Decide {
        decision: DecisionId(1),
        by: Actor::Person,
        chosen: Choice::Other(t("neither: do c")),
        rationale: t("both options assume the daemon exists"),
    });
    let d = &h.state.decisions[&DecisionId(1)];
    let DecisionState::Taken { chosen, at, .. } = &d.state else {
        panic!("taken");
    };
    assert_eq!(*chosen, d.options, "one past the offered options");
    let taken = h.log.iter().find(|e| e.seq == *at).unwrap();
    assert_eq!(taken.text_field("other").unwrap().as_str(), "neither: do c");
    assert_eq!(h.state.items[&ItemId(1)].state, ItemState::Ready);
}

#[test]
fn a_mechanical_decision_is_taken_at_once_by_its_raiser() {
    let mut h = item_in(ItemStateKind::Claimed);
    let before = h.log.len();
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
    assert_eq!(h.log.len(), before + 2, "raised and taken are two events");
    assert_eq!(
        h.log[before + 1].text_field("rationale").unwrap().as_str(),
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
fn an_auditor_reviews_others_work_once() {
    let mut h = item_in(ItemStateKind::Merged);
    assert_eq!(
        h.run(Command::Audit {
            item: ItemId(1),
            by: agent("heron"),
        }),
        Err(Refusal::AuditOwnWork(ItemId(1))),
        "heron wrote it"
    );
    h.must(Command::Audit {
        item: ItemId(1),
        by: agent("wren"),
    });
    assert_eq!(h.state.items[&ItemId(1)].audited_by, vec![agent("wren")]);
    assert_eq!(
        h.run(Command::Audit {
            item: ItemId(1),
            by: agent("wren"),
        }),
        Err(Refusal::AlreadyAudited {
            item: ItemId(1),
            by: agent("wren")
        })
    );
}

#[test]
fn a_claim_is_released_by_its_holder_or_the_person_only() {
    let mut h = item_in(ItemStateKind::Claimed);
    assert_eq!(
        h.run(Command::Release {
            item: ItemId(1),
            by: Actor::Agent(agent("wren")),
        }),
        Err(Refusal::NotTheClaimant {
            item: ItemId(1),
            holder: agent("heron")
        })
    );
    h.must(Command::Release {
        item: ItemId(1),
        by: Actor::Person,
    });
    assert_eq!(h.state.items[&ItemId(1)].state, ItemState::Ready);
    h.must(Command::Claim {
        item: ItemId(1),
        agent: agent("wren"),
    });
}

fn supersede(item: ItemId, by: ItemId, actor: Actor) -> Command {
    Command::Supersede { item, by, actor }
}

#[test]
fn only_the_person_supersedes_and_only_with_a_live_item() {
    let mut h = item_in(ItemStateKind::Ready);
    let heron = Actor::Agent(agent("heron"));
    assert_eq!(
        h.run(supersede(ItemId(1), ItemId(2), heron)),
        Err(Refusal::SupersedeIsForThePerson(ItemId(1)))
    );
    assert_eq!(
        h.run(supersede(ItemId(1), ItemId(1), Actor::Person)),
        Err(Refusal::NotAReplacement {
            item: ItemId(1),
            by: ItemId(1)
        })
    );
    assert_eq!(
        h.run(supersede(ItemId(1), ItemId(9), Actor::Person)),
        Err(Refusal::UnknownItem(ItemId(9)))
    );
    h.must(supersede(ItemId(1), ItemId(2), Actor::Person));
    assert_eq!(h.state.ready().count(), 1, "the superseded item left ready");
    // A chain cannot loop back: I-2 cannot be replaced by I-1.
    assert_eq!(
        h.run(supersede(ItemId(2), ItemId(1), Actor::Person)),
        Err(Refusal::NotAReplacement {
            item: ItemId(2),
            by: ItemId(1)
        })
    );
}

#[test]
fn a_dependency_on_a_superseded_item_waits_on_its_replacement() {
    let mut h = item_in(ItemStateKind::Ready);
    h.must(Command::ItemAdd {
        goal: t("depends on the first"),
        checks: vec![],
        depends_on: vec![ItemId(1)],
    });
    h.must(supersede(ItemId(1), ItemId(2), Actor::Person));
    assert_eq!(
        h.run(Command::Claim {
            item: ItemId(3),
            agent: agent("wren")
        }),
        Err(Refusal::DependenciesOpen {
            item: ItemId(3),
            waiting_on: vec![ItemId(2)]
        })
    );
    h.must(Command::Claim {
        item: ItemId(2),
        agent: agent("heron"),
    });
    h.must(Command::Done {
        item: ItemId(2),
        by: agent("heron"),
        evidence: vec![evidence(1)],
    });
    h.must(Command::Merge {
        item: ItemId(2),
        commit: CommitHash("abc".into()),
    });
    h.must(Command::Claim {
        item: ItemId(3),
        agent: agent("wren"),
    });
}

/// The reverse index is a second copy of facts the log already
/// holds, so it is checked against a full scan after every command
/// of a long pseudo-random history of adds, dependencies, decisions
/// and replacements, and against a fresh fold of the log.
#[test]
fn the_reverse_index_always_matches_a_scan() {
    for seed in 1..=20u64 {
        let mut rng = seed;
        let mut next = |n: u32| {
            // A linear congruential generator: enough to vary the
            // history, and the same on every run.
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((rng >> 33) % u64::from(n.max(1))) as u32
        };
        let mut h = Harness::new();
        h.must(Command::PhaseOpen { goal: t("index") });
        for _ in 0..60 {
            let items = h.state.items.len() as u32;
            match next(6) {
                0 | 1 => {
                    let depends_on = if items == 0 {
                        vec![]
                    } else {
                        (0..next(3)).map(|_| ItemId(next(items) + 1)).collect()
                    };
                    let mut depends_on: Vec<ItemId> = depends_on;
                    depends_on.sort();
                    depends_on.dedup();
                    h.must(Command::ItemAdd {
                        goal: t("an item"),
                        checks: vec![],
                        depends_on,
                    });
                }
                2 if items > 0 => {
                    let item = ItemId(next(items) + 1);
                    let agent = agent("heron");
                    if h.run(Command::Claim {
                        item,
                        agent: agent.clone(),
                    })
                    .is_ok()
                    {
                        h.must(Command::DecisionRaise {
                            by: Actor::Agent(agent.clone()),
                            about: Some(item),
                            class: DecisionClass::Mechanical,
                            question: t("q"),
                            options: vec![t("a"), t("b")],
                            recommended: 0,
                            rationale: t("r"),
                        });
                        h.must(Command::Release {
                            item,
                            by: Actor::Agent(agent),
                        });
                    }
                }
                3 if items > 1 => {
                    // Refused when the pair is not allowed; either
                    // way the index must still match.
                    let _ = h.run(supersede(
                        ItemId(next(items) + 1),
                        ItemId(next(items) + 1),
                        Actor::Person,
                    ));
                }
                4 if items > 0 => {
                    // Refused when an item cannot take it.
                    let _ = h.run(Command::RaiseAction {
                        by: Actor::Person,
                        what: t("do it"),
                        done_when: t("done"),
                        blocks: (0..next(2)).map(|_| ItemId(next(items) + 1)).collect(),
                        informs: (0..next(3)).map(|_| ItemId(next(items) + 1)).collect(),
                    });
                }
                5 if !h.state.actions.is_empty() => {
                    let n = h.state.actions.len() as u32;
                    let _ = h.run(Command::ActionDone {
                        action: crate::ids::ActionId(next(n) + 1),
                        by: Actor::Person,
                        evidence: vec![],
                        note: Some(t("did it")),
                    });
                }
                _ => {}
            }
            assert_eq!(h.state.check_index(), Ok(()), "seed {seed}");
        }
        assert!(
            !h.state.index.replaced.is_empty()
                && !h.state.index.dependents.is_empty()
                && !h.state.index.actions_about.is_empty(),
            "seed {seed} exercised too little"
        );
        assert_eq!(State::fold(&h.log).unwrap(), h.state, "seed {seed}");
    }
}

#[test]
fn a_wrong_index_is_caught() {
    let mut h = item_in(ItemStateKind::Ready);
    h.must(supersede(ItemId(1), ItemId(2), Actor::Person));
    assert_eq!(h.state.check_index(), Ok(()));
    h.state.index.replaced.clear();
    assert!(h.state.check_index().is_err());
}

#[test]
fn a_paused_seat_finishes_what_it_holds_and_takes_nothing_new() {
    let mut h = item_in(ItemStateKind::Claimed);
    let heron = agent("heron");
    let pause = |actor| Command::Pause {
        seats: vec![agent("heron"), agent("wren")],
        actor,
    };
    assert_eq!(
        h.run(pause(Actor::Agent(heron.clone()))),
        Err(Refusal::SeatControlIsForThePerson)
    );
    h.must(pause(Actor::Person));
    // What it holds, it finishes.
    h.must(Command::Done {
        item: ItemId(1),
        by: heron.clone(),
        evidence: vec![evidence(1)],
    });
    h.must(Command::Merge {
        item: ItemId(1),
        commit: CommitHash("abc".into()),
    });
    // Nothing new: no claim, and no audit for the paused auditor.
    assert_eq!(
        h.run(Command::Claim {
            item: ItemId(2),
            agent: heron.clone()
        }),
        Err(Refusal::SeatPaused(heron.clone()))
    );
    assert_eq!(
        h.run(Command::Audit {
            item: ItemId(1),
            by: agent("wren")
        }),
        Err(Refusal::SeatPaused(agent("wren")))
    );
    h.must(Command::Resume {
        seats: vec![heron.clone()],
        actor: Actor::Person,
    });
    h.must(Command::Claim {
        item: ItemId(2),
        agent: heron,
    });
    assert!(h.state.paused.contains(&agent("wren")));
    assert_eq!(State::fold(&h.log).unwrap(), h.state);
}

#[test]
fn a_person_action_blocks_or_informs_and_only_the_person_does_it() {
    let mut h = item_in(ItemStateKind::Claimed);
    let heron = Actor::Agent(agent("heron"));
    let raise = |by: Actor, blocks: Vec<ItemId>, informs: Vec<ItemId>| Command::RaiseAction {
        by,
        what: t("add the builder seat"),
        done_when: t("ris status lists it"),
        blocks,
        informs,
    };
    // An agent cannot raise one, blocking or not: the log is unchanged.
    let before = h.log.len();
    for (blocks, informs) in [(vec![ItemId(1)], vec![]), (vec![], vec![ItemId(2)])] {
        assert_eq!(
            h.run(raise(heron.clone(), blocks, informs)),
            Err(Refusal::RaiseActionIsForThePerson)
        );
    }
    assert_eq!(h.log.len(), before);
    assert_eq!(
        h.run(Command::RaiseAction {
            by: Actor::Person,
            what: t(" "),
            done_when: t("x"),
            blocks: vec![],
            informs: vec![],
        }),
        Err(Refusal::MalformedAction)
    );
    // The person raises it: I-1 (claimed) is set aside, I-2 proceeds.
    h.must(raise(Actor::Person, vec![ItemId(1)], vec![ItemId(2)]));
    let a = crate::ids::ActionId(1);
    assert_eq!(
        h.state.items[&ItemId(1)].state,
        ItemState::Blocked {
            on: vec![Blocker::Action(a)]
        }
    );
    assert_eq!(h.state.items[&ItemId(2)].state, ItemState::Ready);
    assert_eq!(h.state.actions_about(ItemId(2)), &[a]);
    // A second action also blocks I-1: both must clear.
    h.must(Command::RaiseAction {
        by: Actor::Person,
        what: t("run the trial"),
        done_when: t("the transcript is attached"),
        blocks: vec![ItemId(1)],
        informs: vec![],
    });
    let done = |action, by, note: Option<&str>| Command::ActionDone {
        action,
        by,
        evidence: vec![],
        note: note.map(t),
    };
    assert_eq!(
        h.run(done(a, heron, Some("done"))),
        Err(Refusal::ActionIsForThePerson(a))
    );
    assert_eq!(
        h.run(done(a, Actor::Person, None)),
        Err(Refusal::ActionNeedsResult(a))
    );
    h.must(done(a, Actor::Person, Some("rook added")));
    assert!(matches!(
        h.state.items[&ItemId(1)].state,
        ItemState::Blocked { .. }
    ));
    assert!(matches!(
        h.run(done(a, Actor::Person, Some("again"))),
        Err(Refusal::IllegalActionTransition { .. })
    ));
    h.must(done(crate::ids::ActionId(2), Actor::Person, Some("ran it")));
    assert_eq!(h.state.items[&ItemId(1)].state, ItemState::Ready);
    assert_eq!(h.state.check_index(), Ok(()));
    assert_eq!(State::fold(&h.log).unwrap(), h.state);
}

/// An item blocked on a decision and an action together becomes
/// ready only when both have cleared, in either order.
#[test]
fn a_decision_and_an_action_together_must_both_clear() {
    let a = crate::ids::ActionId(1);
    let d = DecisionId(1);
    for decision_first in [true, false] {
        // I-1 is claimed and set aside on D-1, a substantive decision.
        let mut h = item_in(ItemStateKind::Blocked);
        h.must(Command::RaiseAction {
            by: Actor::Person,
            what: t("run the trial"),
            done_when: t("the transcript is attached"),
            blocks: vec![ItemId(1)],
            informs: vec![],
        });
        assert_eq!(
            h.state.items[&ItemId(1)].state,
            ItemState::Blocked {
                on: vec![Blocker::Decision(d), Blocker::Action(a)]
            }
        );
        let decide = Command::Decide {
            decision: d,
            by: Actor::Person,
            chosen: Choice::Option(0),
            rationale: t("a"),
        };
        let done = Command::ActionDone {
            action: a,
            by: Actor::Person,
            evidence: vec![],
            note: Some(t("ran it")),
        };
        let (first, second, left) = if decision_first {
            (decide, done, Blocker::Action(a))
        } else {
            (done, decide, Blocker::Decision(d))
        };
        h.must(first);
        assert_eq!(
            h.state.items[&ItemId(1)].state,
            ItemState::Blocked { on: vec![left] },
            "decision first: {decision_first}"
        );
        h.must(second);
        assert_eq!(h.state.items[&ItemId(1)].state, ItemState::Ready);
        assert_eq!(h.state.check_index(), Ok(()));
        assert_eq!(State::fold(&h.log).unwrap(), h.state);
    }
}

#[test]
fn a_run_is_started_and_stopped_by_the_person_once_at_a_time() {
    let mut h = with_phase_and_item();
    let start = |actor| Command::RunStart {
        seat: agent("rook"),
        actor,
        runner: t("claude"),
        model: None,
    };
    assert_eq!(
        h.run(start(Actor::Agent(agent("heron")))),
        Err(Refusal::SeatControlIsForThePerson)
    );
    h.must(start(Actor::Person));
    assert!(matches!(
        h.run(start(Actor::Person)),
        Err(Refusal::AlreadyRunning { .. })
    ));
    let stop = Command::RunStop {
        seat: agent("rook"),
        actor: Actor::Person,
        status: Some(0),
    };
    h.must(stop.clone());
    assert_eq!(h.run(stop), Err(Refusal::NotRunning(agent("rook"))));
    h.must(Command::Pause {
        seats: vec![agent("rook")],
        actor: Actor::Person,
    });
    assert_eq!(
        h.run(start(Actor::Person)),
        Err(Refusal::SeatPaused(agent("rook")))
    );
    assert_eq!(State::fold(&h.log).unwrap(), h.state);
}

#[test]
fn dependencies_must_merge_before_a_claim() {
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
fn the_report_hash_follows_the_log_not_the_rendering() {
    let mut h = item_in(ItemStateKind::Claimed);
    let before = report_hash(&Hash::ZERO, &h.state.head);
    // The same log head gives the same hash however it is rendered.
    assert_eq!(before, report_hash(&Hash::ZERO, &h.state.head));
    // One more event changes it.
    h.must(item_command(CommandKind::RaiseMechanical).unwrap());
    let after = report_hash(&Hash::ZERO, &h.state.head);
    assert_ne!(before, after);
    // Removing text from the log does not: the head is unchanged.
    let stripped: Vec<Event> = h.log.iter().cloned().map(Event::redacted).collect();
    assert_eq!(State::fold(&stripped).unwrap().head, h.state.head);
    // Chained: the previous report feeds the next.
    assert_ne!(report_hash(&before, &h.state.head), after);
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
