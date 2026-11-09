//! `ris`: the osiris command-line interface.
//!
//! Every subcommand is a thin shell over the core's `apply` and the
//! store. Output is plain text by default and JSON with `--json`. A
//! refusal exits 2 with the refusal and what to do next on stderr; a
//! failed check exits 3 with the failures and their evidence ids; any
//! other error exits 1.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ris_core::command::Command;
use ris_core::ids::{Actor, AgentName, CheckId, CommitHash, DecisionId, EvidenceId, ItemId};
use ris_core::primitives::{DecisionClass, MessageKind};
use serde_json::json;

use ris::checks;
use ris::project::{CliError, Project};
use ris::render::{self, Output};

#[derive(Parser)]
#[command(
    name = "ris",
    version,
    about = "osiris: decide at the boundaries, build in between"
)]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create .osiris/ in the current directory.
    Init,
    /// Open or close a phase.
    Phase {
        #[command(subcommand)]
        action: PhaseCmd,
    },
    /// Add an item to the open phase.
    Item {
        #[command(subcommand)]
        action: ItemCmd,
    },
    /// Items you may claim now.
    Ready,
    /// Take an item. Needs RIS_AGENT.
    Claim { item: ItemId },
    /// Raise a decision: mechanical ones are taken at once and logged;
    /// substantive ones wait for the person and set the item aside.
    Decision {
        #[command(subcommand)]
        action: DecisionCmd,
    },
    /// Take a queued decision.
    Decide {
        decision: DecisionId,
        /// The option number.
        #[arg(long)]
        choose: usize,
        #[arg(long)]
        rationale: String,
    },
    /// Mark a held item done, attaching evidence.
    Done {
        item: ItemId,
        /// A file to attach as evidence. Repeatable.
        #[arg(long = "evidence")]
        evidence: Vec<PathBuf>,
    },
    /// Record that a done item's commit is on the phase's branch.
    Merge {
        item: ItemId,
        /// The commit; defaults to git HEAD in the project root.
        #[arg(long)]
        commit: Option<String>,
    },
    /// The person's decision queue, or an agent's messages and items.
    Inbox,
    /// Send a typed message to an agent.
    Send {
        to: String,
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long)]
        about: Option<ItemId>,
        body: String,
    },
    /// The phase report.
    Report,
    /// Verify the chain and print the head.
    Verify,
    /// Dump the log as JSON lines, from a sequence number.
    Log {
        #[arg(long, default_value_t = 1)]
        from: u64,
    },
}

#[derive(Subcommand)]
enum PhaseCmd {
    Open { goal: String },
    Close,
}

#[derive(Subcommand)]
enum ItemCmd {
    Add(ItemAdd),
}

#[derive(Args)]
struct ItemAdd {
    goal: String,
    /// A check from .osiris/checks.toml the item must pass. Repeatable.
    #[arg(long = "check")]
    checks: Vec<String>,
    /// An item this one cannot start before. Repeatable.
    #[arg(long = "depends")]
    depends_on: Vec<ItemId>,
}

#[derive(Subcommand)]
enum DecisionCmd {
    Raise(DecisionRaise),
}

#[derive(Args)]
struct DecisionRaise {
    #[arg(long)]
    question: String,
    /// An option. Give at least two.
    #[arg(long = "option", required = true)]
    options: Vec<String>,
    /// The recommended option's number.
    #[arg(long, default_value_t = 0)]
    recommend: usize,
    /// The item this is about.
    #[arg(long)]
    about: Option<ItemId>,
    /// Expensive to reverse: queue it for the person.
    #[arg(long)]
    substantive: bool,
    /// Why the recommendation holds. Required for a mechanical decision.
    #[arg(long, default_value = "")]
    rationale: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Handoff,
    InterfaceChanged,
    Question,
    Finding,
}

impl From<Kind> for MessageKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Handoff => MessageKind::Handoff,
            Kind::InterfaceChanged => MessageKind::InterfaceChanged,
            Kind::Question => MessageKind::Question,
            Kind::Finding => MessageKind::Finding,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(out) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&out.json).unwrap());
            } else {
                println!("{}", out.text);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if cli.json {
                let (kind, detail) = match &e {
                    CliError::Refused(r) => {
                        ("refused", json!({ "refusal": r, "next": r.next_step() }))
                    }
                    other => ("error", json!({ "message": other.to_string() })),
                };
                eprintln!("{}", json!({ kind: detail }));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(match e {
                CliError::Refused(_) => 2,
                CliError::ChecksFailed { .. } => 3,
                _ => 1,
            })
        }
    }
}

fn agent_only(actor: &Actor) -> Result<AgentName, CliError> {
    match actor {
        Actor::Agent(name) => Ok(name.clone()),
        Actor::Person => Err(CliError::Other(
            "this command is an agent's: set RIS_AGENT=<name>".into(),
        )),
    }
}

fn run(cli: &Cli) -> Result<Output, CliError> {
    let cwd = std::env::current_dir()?;
    if let Cmd::Init = cli.command {
        let p = Project::init(&cwd)?;
        return Ok(Output::new(
            format!(
                "initialised {}/.osiris\n  next: edit .osiris/config.toml (agents, denylist), then `ris phase open <goal>`",
                p.root.display()
            ),
            json!({ "root": p.root }),
        ));
    }
    let mut p = Project::find(&cwd)?;
    let actor = p.actor()?;

    match &cli.command {
        Cmd::Init => unreachable!(),

        Cmd::Phase { action } => match action {
            PhaseCmd::Open { goal } => {
                let goal = p.text("goal", goal)?;
                let (l, _) = p.run(&Command::PhaseOpen { goal })?;
                let phase = l.state.open_phase().expect("just opened");
                Ok(Output::new(
                    format!("opened {}", phase.id),
                    json!({ "phase": phase.id.to_string() }),
                ))
            }
            PhaseCmd::Close => {
                let (l, events) = p.run(&Command::PhaseClose)?;
                let closed = events.last().expect("close emits one event");
                let report = match &closed.structure {
                    ris_core::event::Structure::PhaseClosed { report, .. } => report.to_string(),
                    _ => unreachable!(),
                };
                let out = render::report(&l);
                Ok(Output::new(
                    out.text,
                    json!({ "report": report, "phase": out.json }),
                ))
            }
        },

        Cmd::Item { action } => match action {
            ItemCmd::Add(a) => {
                let goal = p.text("goal", &a.goal)?;
                for c in &a.checks {
                    if !p.checks.checks.contains_key(c) {
                        return Err(CliError::Config(format!(
                            "check `{c}` is not in .osiris/checks.toml"
                        )));
                    }
                }
                let (l, _) = p.run(&Command::ItemAdd {
                    goal,
                    checks: a.checks.iter().cloned().map(CheckId).collect(),
                    depends_on: a.depends_on.clone(),
                })?;
                let id = l.state.items.keys().last().expect("just added");
                Ok(Output::new(
                    format!("added {id}"),
                    json!({ "item": id.to_string() }),
                ))
            }
        },

        Cmd::Ready => Ok(render::ready(&p.load()?)),

        Cmd::Claim { item } => {
            let agent = agent_only(&actor)?;
            let (l, _) = p.run(&Command::Claim { item: *item, agent })?;
            let i = &l.state.items[item];
            let checks: Vec<String> = i.checks.iter().map(ToString::to_string).collect();
            Ok(Output::new(
                format!(
                    "claimed {item}: {}\n  checks: {}",
                    l.text(i.at, "goal"),
                    if checks.is_empty() {
                        "none".to_owned()
                    } else {
                        checks.join(", ")
                    }
                ),
                json!({ "item": item.to_string(), "goal": l.text(i.at, "goal"), "checks": checks }),
            ))
        }

        Cmd::Decision { action } => match action {
            DecisionCmd::Raise(d) => {
                let class = if d.substantive {
                    DecisionClass::Substantive
                } else {
                    DecisionClass::Mechanical
                };
                if class == DecisionClass::Mechanical && d.rationale.trim().is_empty() {
                    return Err(CliError::Other(
                        "a mechanical decision needs --rationale: it is taken at once".into(),
                    ));
                }
                let question = p.text("question", &d.question)?;
                let mut options = Vec::new();
                for (n, o) in d.options.iter().enumerate() {
                    options.push(p.text(&format!("option.{n}"), o)?);
                }
                let rationale = p.text("rationale", &d.rationale)?;
                let (l, _) = p.run(&Command::DecisionRaise {
                    by: actor.clone(),
                    about: d.about,
                    class,
                    question,
                    options,
                    recommended: d.recommend,
                    rationale,
                })?;
                let id = l.state.decisions.keys().last().expect("just raised");
                let text = match class {
                    DecisionClass::Mechanical => format!("{id} taken: option {}", d.recommend),
                    DecisionClass::Substantive => format!(
                        "{id} queued for the person{}",
                        d.about
                            .map(|i| format!("; {i} is set aside, take another"))
                            .unwrap_or_default()
                    ),
                };
                Ok(Output::new(text, json!({ "decision": id.to_string() })))
            }
        },

        Cmd::Decide {
            decision,
            choose,
            rationale,
        } => {
            let rationale = p.text("rationale", rationale)?;
            let (l, events) = p.run(&Command::Decide {
                decision: *decision,
                by: actor.clone(),
                chosen: *choose,
                rationale,
            })?;
            let unblocked: Vec<String> = events
                .iter()
                .filter_map(|e| match &e.structure {
                    ris_core::event::Structure::ItemReady { item } => Some(item.to_string()),
                    _ => None,
                })
                .collect();
            let _ = l;
            Ok(Output::new(
                format!(
                    "{decision} taken: option {choose}{}",
                    if unblocked.is_empty() {
                        String::new()
                    } else {
                        format!("; ready again: {}", unblocked.join(", "))
                    }
                ),
                json!({ "decision": decision.to_string(), "unblocked": unblocked }),
            ))
        }

        Cmd::Done { item, evidence } => {
            let agent = agent_only(&actor)?;
            let l = p.load()?;
            let names: Vec<String> = l
                .state
                .items
                .get(item)
                .ok_or(CliError::Refused(ris_core::Refusal::UnknownItem(*item)))?
                .checks
                .iter()
                .map(ToString::to_string)
                .collect();
            let CheckRuns {
                lines,
                evidence: mut ids,
                failed,
            } = run_checks(&mut p, *item, &names)?;
            for file in evidence {
                ids.push(p.evidence.put_file(file)?);
            }
            if !failed.is_empty() {
                return Err(CliError::ChecksFailed {
                    item: *item,
                    stage: "done",
                    failed,
                    lines,
                });
            }
            let (_, _) = p.run(&Command::Done {
                item: *item,
                by: agent,
                evidence: ids.clone(),
            })?;
            let mut text = lines.join("\n");
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!(
                "{item} done with {} piece(s) of evidence\n  next: commit with `Item: {item}` as a trailer, then `ris merge {item}`",
                ids.len()
            ));
            Ok(Output::new(
                text,
                json!({ "item": item.to_string(), "evidence": ids.iter().map(ToString::to_string).collect::<Vec<_>>() }),
            ))
        }

        Cmd::Merge { item, commit } => {
            // The item must be done before anything runs or resolves;
            // the core produces the refusal.
            let l = p.load()?;
            if !matches!(
                l.state.items.get(item).map(|i| &i.state),
                Some(ris_core::primitives::ItemState::Done { .. })
            ) {
                p.run(&Command::Merge {
                    item: *item,
                    commit: CommitHash(commit.clone().unwrap_or_default()),
                })?;
                unreachable!("the core refuses a merge of an item that is not done");
            }
            let commit = match commit {
                Some(c) => c.clone(),
                None => checks::git_head(&p.root)?,
            };
            let message = checks::commit_message(&p.root, &commit)?;
            checks::require_trailer(&message, *item, &commit)?;
            // The full checks: every check the project declares.
            let names: Vec<String> = p.checks.checks.keys().cloned().collect();
            let CheckRuns { lines, failed, .. } = run_checks(&mut p, *item, &names)?;
            if !failed.is_empty() {
                return Err(CliError::ChecksFailed {
                    item: *item,
                    stage: "merged",
                    failed,
                    lines,
                });
            }
            let (_, _) = p.run(&Command::Merge {
                item: *item,
                commit: CommitHash(commit.clone()),
            })?;
            let mut text = lines.join("\n");
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("{item} merged at {commit}"));
            Ok(Output::new(
                text,
                json!({ "item": item.to_string(), "commit": commit }),
            ))
        }

        Cmd::Inbox => Ok(render::inbox(&p.load()?, &actor)),

        Cmd::Send {
            to,
            kind,
            about,
            body,
        } => {
            if !p.config.agents.contains_key(to) {
                return Err(CliError::UnknownAgent(to.clone()));
            }
            let body = p.text("body", body)?;
            let (l, _) = p.run(&Command::MessageSend {
                from: actor.clone(),
                to: AgentName(to.clone()),
                kind: (*kind).into(),
                about: *about,
                body,
            })?;
            let id = l.state.messages.last().expect("just sent").id;
            Ok(Output::new(
                format!("sent {id} to {to}"),
                json!({ "message": id.to_string() }),
            ))
        }

        Cmd::Report => Ok(render::report(&p.load()?)),

        Cmd::Verify => {
            let l = p.load()?;
            let v =
                ris_core::event::verify(&l.log).map_err(ris_core::store::StoreError::Corrupt)?;
            Ok(Output::new(
                format!(
                    "ok: {} events, head {}{}",
                    v.len,
                    v.head,
                    if v.missing_text.is_empty() {
                        String::new()
                    } else {
                        format!("; text removed from {} event(s)", v.missing_text.len())
                    }
                ),
                json!({ "len": v.len, "head": v.head.to_string(), "missing_text": v.missing_text }),
            ))
        }

        Cmd::Log { from } => {
            let l = p.load()?;
            let events: Vec<_> = l.log.iter().filter(|e| e.seq >= *from).collect();
            let lines: Vec<String> = events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap())
                .collect();
            Ok(Output::new(lines.join("\n"), json!(events)))
        }
    }
}

/// Run the named checks against `item`, recording each run in the log
/// and returning the lines to print, the evidence ids, and the names
/// and evidence of any that failed.
/// What running an item's checks produced.
struct CheckRuns {
    /// One line per check, for printing.
    lines: Vec<String>,
    /// Every run's evidence, passed or not.
    evidence: Vec<EvidenceId>,
    /// The checks that failed, with their evidence.
    failed: Vec<(String, EvidenceId)>,
}

fn run_checks(p: &mut Project, item: ItemId, names: &[String]) -> Result<CheckRuns, CliError> {
    let mut lines = Vec::new();
    let mut ids = Vec::new();
    let mut failed = Vec::new();
    for name in names {
        let Some(check) = p.checks.checks.get(name) else {
            return Err(CliError::Config(format!(
                "check `{name}` is not in .osiris/checks.toml"
            )));
        };
        let check = check.clone();
        let outcome = checks::run_check(&p.root, name, &check, &p.evidence)?;
        p.run(&Command::CheckRan {
            item,
            check: CheckId(name.clone()),
            passed: outcome.passed,
            evidence: outcome.evidence,
        })?;
        lines.push(format!(
            "{item}: check {name} {} ({})",
            if outcome.passed { "passed" } else { "FAILED" },
            outcome.evidence
        ));
        ids.push(outcome.evidence);
        if !outcome.passed {
            failed.push((outcome.name, outcome.evidence));
        }
    }
    Ok(CheckRuns {
        lines,
        evidence: ids,
        failed,
    })
}
