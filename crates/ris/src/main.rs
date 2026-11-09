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
use ris_core::command::{Choice, Command};
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
    about = "osiris: decide at the boundaries, build in between",
    after_help = "Ids: I-n item (a unit of work, one commit), D-n decision, P-n phase, \
                  M-n message, E-<hash> evidence. `ris show <id>` prints any of them."
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
    /// Create .osiris/ in the current directory, and OSIRIS.md if absent.
    Init {
        /// Change nothing tracked: rules inside .osiris/, ignore lines
        /// in .git/info/exclude, CLAUDE.local.md importing the rules.
        #[arg(long)]
        local: bool,
    },
    /// Create a builder's worktree beside the repository, on its own
    /// branch from the phase branch, with the rules imported.
    Worktree { seat: String },
    /// Print the agents' rules (what OSIRIS.md holds).
    Rules,
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
    /// Return a claimed item to ready: its holder, or the person.
    Release { item: ItemId },
    /// Raise a decision: mechanical ones are taken at once and logged;
    /// substantive ones wait for the person and set the item aside.
    Decision {
        #[command(subcommand)]
        action: DecisionCmd,
    },
    /// Take a queued decision: one of its options, or your own answer.
    Decide {
        decision: DecisionId,
        /// The option number.
        #[arg(long, conflicts_with = "other", required_unless_present = "other")]
        choose: Option<usize>,
        /// An answer that is not among the options.
        #[arg(long)]
        other: Option<String>,
        #[arg(long)]
        rationale: String,
    },
    /// Run the project's checks here, recording nothing.
    Check {
        /// Only these checks; all of them when none are named.
        names: Vec<String>,
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
    /// One record in full: an item, decision, phase or message by id.
    Show { id: String },
    /// The check-in view: decisions, report and what is ready, in one screen.
    Summary,
    /// The phase report.
    Report,
    /// Scan tracked files (or the given paths) as `ris merge` scans a
    /// commit: exit 2 on a hit, naming the place and never the text.
    Scan { paths: Vec<String> },
    /// Verify the chain and print the head.
    Verify,
    /// Write each closed phase's events to <dir>/P-n.jsonl, scanned
    /// first. Nothing is exported unless you run this.
    Export { dir: PathBuf },
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
    ris::style::init(cli.json);
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
                let text = e.to_string();
                // Colour the leading "refused:" when styling is on.
                match text.strip_prefix("refused:") {
                    Some(rest) if ris::style::on() => {
                        eprintln!("{}{rest}", ris::style::failed("refused:"));
                    }
                    _ => eprintln!("{text}"),
                }
            }
            ExitCode::from(match e {
                CliError::Refused(_) | CliError::Leak { .. } => 2,
                CliError::ChecksFailed { .. } | CliError::CheckFailed { .. } => 3,
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
    if let Cmd::Init { local } = cli.command {
        let p = Project::init(&cwd, local)?;
        let text = if local {
            "initialised .osiris with the rules inside it; nothing tracked changed\n  \
             (.osiris/ and CLAUDE.local.md are excluded in .git/info/exclude;\n   \
             CLAUDE.local.md imports .osiris/OSIRIS.md unless it already existed)\n  \
             next: set the denylist and seats in .osiris/config.toml and checks in .osiris/checks.toml,\n        \
             then `ris phase open <goal>`"
        } else {
            "initialised .osiris and wrote OSIRIS.md (the agents' rules)\n  \
             next: point your agents' rules file at OSIRIS.md (for example `@OSIRIS.md` in CLAUDE.md),\n        \
             set the denylist and seats in .osiris/config.toml and checks in .osiris/checks.toml,\n        \
             then `ris phase open <goal>`"
        };
        return Ok(Output::new(text, json!({ "root": p.root, "local": local })));
    }
    if let Cmd::Rules = cli.command {
        return Ok(Output::new(
            ris::project::RULES.trim_end(),
            json!({ "rules": ris::project::RULES }),
        ));
    }
    let mut p = Project::find(&cwd)?;
    let actor = p.actor()?;

    match &cli.command {
        Cmd::Init { .. } | Cmd::Rules => unreachable!(),

        Cmd::Worktree { seat } => {
            if !p.config.agents.contains_key(seat) {
                return Err(CliError::UnknownAgent(seat.clone()));
            }
            let name = p
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into());
            let path = p
                .root
                .parent()
                .unwrap_or(&p.root)
                .join(format!("{name}-{seat}"));
            if path.exists() {
                return Err(CliError::Other(format!(
                    "{} already exists\n  next: start the agent there, or remove it with `git worktree remove`",
                    path.display()
                )));
            }
            let has_branch = git(
                &p.root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{seat}"),
                ],
            )
            .is_ok();
            let target = path.to_string_lossy().into_owned();
            let args: Vec<&str> = if has_branch {
                vec!["worktree", "add", "--quiet", &target, seat]
            } else {
                vec!["worktree", "add", "--quiet", "-b", seat, &target]
            };
            git(&p.root, &args).map_err(|e| CliError::Other(format!("git worktree add: {e}")))?;
            ris::project::exclude_locally(&p.root, &[ris::project::LOCAL_IMPORT_FILE])?;
            let import = path.join(ris::project::LOCAL_IMPORT_FILE);
            if !import.exists() {
                std::fs::write(&import, format!("@{}\n", p.rules_path().display()))?;
            }
            Ok(Output::new(
                format!(
                    "worktree for {seat} at {}\n  next: cd there and start the agent with RIS_AGENT={seat}",
                    path.display()
                ),
                json!({ "seat": seat, "path": path }),
            ))
        }

        Cmd::Phase { action } => match action {
            PhaseCmd::Open { goal } => {
                let goal = p.text("goal", goal)?;
                let command = Command::PhaseOpen { goal };
                // Refusals first, so a refused open creates no branch.
                let l = p.load()?;
                ris_core::apply(&l.state, &command)?;
                if p.config.denylist.is_empty() && !p.config.confirm_empty_denylist {
                    return Err(CliError::Config(
                        "the denylist in .osiris/config.toml is empty; text is scanned as it is \
                         written and the log cannot be redacted, so fill it before work starts\n  \
                         next: add your name, the client's and product names, your home directory; \
                         or set confirm_empty_denylist = true if empty is deliberate"
                            .into(),
                    ));
                }
                let branch = open_phase_branch(&p, l.state.next_phase_id())?;
                let (l, _) = p.run(&command)?;
                let phase = l.state.open_phase().expect("just opened");
                let text = match &branch {
                    Some(b) => format!("opened {} on branch {b}", phase.id),
                    None => format!("opened {}", phase.id),
                };
                Ok(Output::new(
                    text,
                    json!({ "phase": phase.id.to_string(), "branch": branch }),
                ))
            }
            PhaseCmd::Close => {
                let l = p.load()?;
                let phase = l
                    .state
                    .open_phase()
                    .ok_or(CliError::Refused(ris_core::Refusal::NoOpenPhase))?;
                let (phase_id, goal) = (phase.id, l.text(phase.at, "goal").to_owned());
                let closing = closing_merge(&mut p, phase_id, &goal)?;
                let (l, events) = p.run(&Command::PhaseClose)?;
                let closed = events.last().expect("close emits one event");
                let report = match &closed.structure {
                    ris_core::event::Structure::PhaseClosed { report, .. } => report.to_string(),
                    _ => unreachable!(),
                };
                let out = render::report(&l);
                Ok(Output::new(
                    format!("{}\n{closing}", out.text),
                    json!({ "report": report, "phase": out.json, "closing_merge": closing }),
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
            other,
            rationale,
        } => {
            let rationale = p.text("rationale", rationale)?;
            let chosen = match (choose, other) {
                (Some(n), None) => Choice::Option(*n),
                (None, Some(answer)) => Choice::Other(p.text("other", answer)?),
                _ => unreachable!("clap requires exactly one of --choose and --other"),
            };
            let (l, events) = p.run(&Command::Decide {
                decision: *decision,
                by: actor.clone(),
                chosen,
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
            let picked = match choose {
                Some(n) => format!("option {n}"),
                None => "your own answer, recorded as a new option".to_owned(),
            };
            Ok(Output::new(
                format!(
                    "{decision} taken: {picked}{}",
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
            let commit_hint = if p.config.trailers {
                format!(
                    "commit with `Item: {item}` and `Agent: {}` as trailers",
                    p.actor()?.to_string().trim_start_matches("agent ")
                )
            } else {
                "commit it as one commit".to_owned()
            };
            text.push_str(&format!(
                "{item} done with {} piece(s) of evidence\n  next: {commit_hint}, then `ris merge {item}`",
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
            let author = last_claimant(&l, *item)
                .ok_or_else(|| CliError::Other(format!("{item} has no claim in the log")))?;
            let (commit, lines) = if same_dir(&p.work, &p.root) {
                // Width one: the commit is already on the phase branch.
                let commit = match commit {
                    Some(c) => c.clone(),
                    None => checks::git_head(&p.work)?,
                };
                let lines = vet_and_check(&mut p, *item, &author, &commit)?;
                (commit, lines)
            } else {
                if commit.is_some() {
                    return Err(CliError::Other(
                        "--commit is for width one; from a worktree, merge commits HEAD".into(),
                    ));
                }
                merge_from_worktree(&mut p, *item, &author)?
            };
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

        Cmd::Check { names } => {
            let names: Vec<String> = if names.is_empty() {
                p.checks.checks.keys().cloned().collect()
            } else {
                names.clone()
            };
            let mut lines = Vec::new();
            let mut failed = false;
            let mut results = Vec::new();
            for name in &names {
                let check = p.checks.checks.get(name).cloned().ok_or_else(|| {
                    CliError::Config(format!("check `{name}` is not in .osiris/checks.toml"))
                })?;
                let (passed, output) = checks::run_check_quietly(&p.work, &check)?;
                if passed {
                    lines.push(format!("{name}: {}", ris::style::passed("passed")));
                } else {
                    failed = true;
                    lines.push(format!("{name}: {}", ris::style::failed("FAILED")));
                    let tail: Vec<&str> = output.lines().rev().take(20).collect();
                    for l in tail.into_iter().rev() {
                        lines.push(format!("  | {l}"));
                    }
                }
                results.push(json!({ "check": name, "passed": passed }));
            }
            if failed {
                return Err(CliError::CheckFailed { lines });
            }
            Ok(Output::new(lines.join("\n"), json!({ "checks": results })))
        }

        Cmd::Release { item } => {
            p.run(&Command::Release {
                item: *item,
                by: actor.clone(),
            })?;
            Ok(Output::new(
                format!("released {item}; it is ready again"),
                json!({ "item": item.to_string() }),
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

        Cmd::Show { id } => render::show(&p.load()?, id).map_err(CliError::Other),

        Cmd::Summary => Ok(render::summary(&p.load()?, &actor)),

        Cmd::Report => Ok(render::report(&p.load()?)),

        Cmd::Scan { paths } => {
            let files = ris::exits::tracked_files(&p.work, paths)?;
            let hits = ris::exits::scan_files(&p.work, &files, &p.policy)?;
            if !hits.is_empty() {
                return Err(CliError::Leak {
                    what: "the working tree".into(),
                    hits,
                });
            }
            Ok(Output::new(
                format!("clean: {} file(s) scanned", files.len()),
                json!({ "files": files.len(), "hits": [] }),
            ))
        }

        Cmd::Export { dir } => {
            let l = p.load()?;
            let hits = ris::export::rescan(&l.log, &p.policy);
            if !hits.is_empty() {
                return Err(CliError::Other(format!(
                    "refused: the log holds text the current policy refuses; nothing was written\n  {}\n  \
                     next: the text cannot be removed yet (no redact command); keep this log local",
                    hits.join("\n  ")
                )));
            }
            let files = ris::export::phase_files(&l.log)?;
            let new = ris::export::write(dir, &files)?;
            Ok(Output::new(
                format!(
                    "exported {} closed phase(s) to {} ({new} new)",
                    files.len(),
                    dir.display()
                ),
                json!({ "phases": files.iter().map(|f| f.name.clone()).collect::<Vec<_>>(), "new": new }),
            ))
        }

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
        let outcome = checks::run_check(&p.work, name, &check, &p.evidence)?;
        p.run(&Command::CheckRan {
            item,
            check: CheckId(name.clone()),
            passed: outcome.passed,
            evidence: outcome.evidence,
        })?;
        lines.push(format!(
            "{item}: check {name} {} ({})",
            if outcome.passed {
                ris::style::passed("passed")
            } else {
                ris::style::failed("FAILED")
            },
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

/// The agent that last claimed `item`, from the log: the item's author.
fn last_claimant(l: &ris::project::Loaded, item: ItemId) -> Option<String> {
    l.log.iter().rev().find_map(|e| match &e.structure {
        ris_core::event::Structure::ItemClaimed { item: i, agent } if *i == item => {
            Some(agent.to_string())
        }
        _ => None,
    })
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Everything a commit must pass before it merges: its trailers, the
/// exit scan, and every project check, run in the caller's worktree.
/// Returns the per-check lines.
fn vet_and_check(
    p: &mut Project,
    item: ItemId,
    author: &str,
    commit: &str,
) -> Result<Vec<String>, CliError> {
    let message = checks::commit_message(&p.work, commit)?;
    if p.config.trailers {
        checks::require_trailer(&message, item, commit)?;
        checks::require_agent_trailer(&message, author, commit)?;
    }
    let hits = ris::exits::scan_commit(&p.work, commit, &message, &p.policy)?;
    if !hits.is_empty() {
        return Err(CliError::Leak {
            what: format!("commit {}", &commit[..commit.len().min(12)]),
            hits,
        });
    }
    let names: Vec<String> = p.checks.checks.keys().cloned().collect();
    let CheckRuns { lines, failed, .. } = run_checks(p, item, &names)?;
    if !failed.is_empty() {
        return Err(CliError::ChecksFailed {
            item,
            stage: "merged",
            failed,
            lines,
        });
    }
    Ok(lines)
}

/// Run git in `dir`; the trimmed stdout, or the trimmed stderr as the
/// error.
fn git(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Width above one. Under the merge lock: rebase the builder's branch
/// onto the phase branch, require exactly one commit on top, vet it and
/// run every check on the combined tree, then fast-forward the phase
/// branch. Nothing moves unless every step passes.
fn merge_from_worktree(
    p: &mut Project,
    item: ItemId,
    author: &str,
) -> Result<(String, Vec<String>), CliError> {
    let _lock = ris_store_sqlite::MergeLock::acquire(
        &p.root.join(ris::project::DIR).join("merge.lock"),
        std::time::Duration::from_secs(30 * 60),
    )?;
    let phase = git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(CliError::Other)?;
    if phase == "HEAD" {
        return Err(CliError::Other(
            "the main worktree is detached; check out the phase branch there".into(),
        ));
    }
    if let Err(e) = git(&p.work, &["rebase", "--quiet", &phase]) {
        let _ = git(&p.work, &["rebase", "--abort"]);
        return Err(CliError::Other(format!(
            "refused: {item} conflicts with work merged to {phase} before it ({})\n  \
             next: `git rebase {phase}` in your worktree, resolve, keep one commit, and run `ris merge {item}` again",
            e.lines().next().unwrap_or("rebase failed")
        )));
    }
    let count = git(&p.work, &["rev-list", "--count", &format!("{phase}..HEAD")])
        .map_err(CliError::Other)?;
    match count.as_str() {
        "1" => {}
        "0" => {
            return Err(CliError::Other(format!(
                "nothing to merge: your branch has no commit beyond {phase}\n  next: commit the item first"
            )));
        }
        n => {
            return Err(CliError::Other(format!(
                "your branch has {n} commits beyond {phase}; an item merges as one\n  \
                 next: squash them (`git reset --soft {phase}` then commit with the trailers)"
            )));
        }
    }
    let commit = checks::git_head(&p.work)?;
    let lines = vet_and_check(p, item, author, &commit)?;
    git(&p.root, &["merge", "--quiet", "--ff-only", &commit]).map_err(|e| {
        CliError::Other(format!(
            "could not move {phase} to {commit}: {e}\n  next: the main worktree may have uncommitted changes in the way"
        ))
    })?;
    Ok((commit, lines))
}

/// The closing merge: the phase branch into the main line, `--no-ff`,
/// in the main worktree, with every check run on the merged result
/// before the phase closes. When the phase's work is already on the
/// main line there is nothing to merge. On a conflict or a failing
/// check the merge is undone (it was never pushed) and the phase stays
/// open. Returns a line saying what happened.
fn closing_merge(
    p: &mut Project,
    phase: ris_core::ids::PhaseId,
    goal: &str,
) -> Result<String, CliError> {
    let _lock = ris_store_sqlite::MergeLock::acquire(
        &p.root.join(ris::project::DIR).join("merge.lock"),
        std::time::Duration::from_secs(30 * 60),
    )?;
    if git(&p.root, &["rev-parse", "--git-dir"]).is_err() {
        return Ok("closing merge: none; the project is not a git repository".to_owned());
    }
    let main = p.config.main_line.clone();
    let branch = git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(CliError::Other)?;
    if branch == main {
        return Ok(format!(
            "closing merge: none needed; the phase's work is already on {main}"
        ));
    }
    if branch == "HEAD" {
        return Err(CliError::Other(
            "the main worktree is detached; check out the phase branch there".into(),
        ));
    }
    if !p.config.merge_on_close {
        return Ok(format!(
            "closing merge: left to the git server (merge_on_close = false); \
             push {branch} and merge it into {main} there"
        ));
    }
    let dirty = git(&p.root, &["status", "--porcelain", "--untracked-files=no"])
        .map_err(CliError::Other)?;
    if !dirty.is_empty() {
        return Err(CliError::Other(format!(
            "the main worktree has uncommitted changes; commit or stash them before closing\n  \
             next: the phase stays open until {branch} can merge into {main}"
        )));
    }
    git(&p.root, &["switch", "--quiet", &main])
        .map_err(|e| CliError::Other(format!("cannot switch to the main line {main}: {e}")))?;
    let undo = |p: &Project| {
        let _ = git(&p.root, &["merge", "--abort"]);
        let _ = git(&p.root, &["reset", "--quiet", "--hard", "ORIG_HEAD"]);
        let _ = git(&p.root, &["switch", "--quiet", &branch]);
    };
    let message = format!("chore: close {phase}: {goal}");
    if let Err(e) = git(
        &p.root,
        &["merge", "--quiet", "--no-ff", "-m", &message, &branch],
    ) {
        let _ = git(&p.root, &["merge", "--abort"]);
        let _ = git(&p.root, &["switch", "--quiet", &branch]);
        return Err(CliError::Other(format!(
            "refused: {branch} conflicts with {main}, which moved during the phase ({})\n  \
             next: merge {main} into {branch}, resolve, merge that as an item, and close again",
            e.lines().next().unwrap_or("merge failed")
        )));
    }
    // The full checks on the merged result, in the main worktree.
    let mut failed = Vec::new();
    let names: Vec<String> = p.checks.checks.keys().cloned().collect();
    for name in &names {
        let check = p.checks.checks[name].clone();
        let (passed, _) = checks::run_check_quietly(&p.root, &check)?;
        if !passed {
            failed.push(name.clone());
        }
    }
    if !failed.is_empty() {
        undo(p);
        return Err(CliError::Other(format!(
            "refused: {branch} merged into {main} fails {}; the merge was undone and the phase stays open\n  \
             next: `ris check` after merging {main} into {branch} shows the failure",
            failed.join(", ")
        )));
    }
    let head = git(&p.root, &["rev-parse", "HEAD"]).map_err(CliError::Other)?;
    // A phase branch the harness made is deleted once merged; `-d`
    // refuses if it were somehow not merged.
    let deleted = if p.config.phase_branches && branch == phase_branch_name(phase) {
        git(&p.root, &["branch", "--quiet", "-d", &branch]).is_ok()
    } else {
        false
    };
    Ok(format!(
        "closing merge: {branch} into {main} at {head}; the main worktree is now on {main}{}",
        if deleted {
            format!("; {branch} deleted")
        } else {
            String::new()
        }
    ))
}

fn phase_branch_name(phase: ris_core::ids::PhaseId) -> String {
    format!("phase/{phase}")
}

/// With phase branches on, create `phase/P-n` from the main line and
/// switch the main worktree to it. Returns the branch, or `None` when
/// branches are off or the project is not a git repository.
fn open_phase_branch(
    p: &Project,
    phase: ris_core::ids::PhaseId,
) -> Result<Option<String>, CliError> {
    if !p.config.phase_branches || git(&p.root, &["rev-parse", "--git-dir"]).is_err() {
        return Ok(None);
    }
    let main = &p.config.main_line;
    let name = phase_branch_name(phase);
    if git(
        &p.root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{main}"),
        ],
    )
    .is_err()
    {
        return Err(CliError::Other(format!(
            "the main line `{main}` has no commits yet, so there is nothing to branch from\n  \
             next: commit on {main} first, set main_line in .osiris/config.toml, \
             or set phase_branches = false"
        )));
    }
    let dirty = git(&p.root, &["status", "--porcelain", "--untracked-files=no"])
        .map_err(CliError::Other)?;
    if !dirty.is_empty() {
        return Err(CliError::Other(format!(
            "the main worktree has uncommitted changes; commit or stash them before opening a phase\n  \
             next: {name} would be created from {main} and checked out there"
        )));
    }
    git(&p.root, &["switch", "--quiet", "-c", &name, main])
        .map_err(|e| CliError::Other(format!("could not create {name} from {main}: {e}")))?;
    Ok(Some(name))
}
