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
use ris::schema;

#[derive(Parser)]
#[command(
    name = "ris",
    version,
    disable_version_flag = true,
    disable_help_subcommand = true,
    about = "osiris: decide at the boundaries, build in between",
    after_help = "Ids: I-n item (a unit of work, one commit), D-n decision, A-n person action, \
                  P-n phase, M-n message, E-<hash> evidence. `ris show <id>` prints any of them."
)]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Print the version. It works as it always has and is left out of
    /// the help, which must fit one screen: nobody reads the help to
    /// find it.
    #[arg(short = 'V', long, action = clap::ArgAction::Version, hide = true)]
    version: (),
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
    /// branch from the phase branch, with the rules imported. Done by
    /// `ris up`; not listed in help.
    #[command(hide = true)]
    Worktree { seat: String },
    /// Make the harness match .osiris/config.toml, doing only the
    /// difference: worktrees, idle worktrees fast-forwarded to the phase
    /// branch, resumes, and agents started in the background. Safe to
    /// run again. The person's.
    Up {
        /// Only these seats; all of them when none is named.
        seats: Vec<String>,
    },
    /// Bring the harness down without losing work: each seat finishes
    /// the item it holds, then its agent stops. Safe to run again.
    Down {
        /// Only these seats; all of them when none is named.
        seats: Vec<String>,
    },
    /// A seat's latest run output, as text. With no seat, every seat's,
    /// each line prefixed with its seat.
    Logs {
        seat: Option<String>,
        /// Keep printing until the run stops; with no seat, until no
        /// seat is running.
        #[arg(long, short)]
        follow: bool,
    },
    /// Print the agents' rules (what OSIRIS.md holds). Not listed in
    /// help: named wherever it is needed.
    #[command(hide = true)]
    Rules,
    /// Add an agent seat to .osiris/config.toml (the person's), or
    /// report a fault in your own seat's setup (`ris seat fault`).
    Seat {
        #[command(subcommand)]
        action: SeatCmd,
    },
    /// Open or close a phase.
    Phase {
        #[command(subcommand)]
        action: PhaseCmd,
    },
    /// Add an item to the open phase, or supersede one.
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
    /// The review queue, the open phase's items; with an item, record
    /// that you reviewed it; with --file, record how closely that
    /// file's change should be read, for the map.
    Audit {
        item: Option<ItemId>,
        /// Every phase's items, not only the open phase's.
        #[arg(long, conflicts_with = "item")]
        all: bool,
        /// A file the open phase changed, as `ris map` lists it: record
        /// your judgement of its risk. Again for the same file, the
        /// later one stands.
        #[arg(long, value_name = "PATH", requires_all = ["risk", "why"], conflicts_with_all = ["item", "all"])]
        file: Option<String>,
        /// The level you judge the file's change to be.
        #[arg(long, value_enum, value_name = "LEVEL", requires = "file")]
        risk: Option<Level>,
        /// One line of why.
        #[arg(long, requires = "file")]
        why: Option<String>,
    },
    /// Something only the person can do: raise one, or record it done.
    Action {
        #[command(subcommand)]
        action: ActionCmd,
    },
    /// Raise a decision: mechanical ones are taken at once and logged;
    /// substantive ones wait for the person and set the item aside.
    Decision {
        #[command(subcommand)]
        action: DecisionCmd,
    },
    /// A decision's full case: what it is about, the pros and cons of
    /// each option, and why the recommendation is what it is.
    Eli5 { decision: DecisionId },
    /// Take a queued decision: one of its options, or your own answer.
    /// With no decision, walk the queue of open substantive decisions,
    /// one per screen: a key picks an option, o gives your own answer,
    /// Enter skips the rationale.
    /// With --json and no decision, print the queue.
    Decide {
        decision: Option<DecisionId>,
        /// The option number.
        #[arg(long, conflicts_with = "other", requires = "decision")]
        choose: Option<usize>,
        /// An answer that is not among the options.
        #[arg(long, requires = "decision")]
        other: Option<String>,
        /// Why; optional.
        #[arg(long, requires = "decision", default_value = "")]
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
    /// One picture of the open phase: its items, what they wait on and the open decisions on them.
    Summary {
        /// Every phase's work, not only the open phase's.
        #[arg(long)]
        all: bool,
    },
    /// The phase report.
    Report,
    /// The phase's change set as a map of the source code: every source
    /// file with the lines the phase added and removed, and how the
    /// files connect. With no phase named, the open phase, or the last
    /// one closed.
    Map {
        /// The phase, such as P-3.
        phase: Option<ris_core::ids::PhaseId>,
        /// Write one self-contained page that draws the map here.
        #[arg(long, value_name = "FILE")]
        html: Option<PathBuf>,
    },
    /// Where each seat is: role, worktree, branch, claim, commits ahead
    /// of and behind the phase branch, uncommitted changes.
    Status,
    /// Scan the files git tracks or would pick up (or the given paths,
    /// tracked or not) as `ris merge` scans a commit: exit 2 on a hit,
    /// naming the place and never the text. Scanning no files is an
    /// error, not a clean result.
    Scan { paths: Vec<String> },
    /// Verify the chain and print the head.
    Verify,
    /// Print the JSON Schema of the `--json` contract: the envelope,
    /// the error, and every command's data. Not listed in help, which
    /// must fit one screen: `docs/json.md` names it for the clients
    /// that validate against it.
    #[command(hide = true)]
    Schema,
    /// Build the last merged commit (the last build that passed every
    /// check) and install it where `[install]` in config.toml says. Also
    /// done on the person's next command after a merge, so not listed
    /// in help. The person's.
    #[command(hide = true)]
    Install,
    /// Import items, standing decisions and open questions from a TOML
    /// file into the open phase. The whole file is validated first; see `ris import
    /// --help` and the onboarding skill for the format.
    Import {
        file: PathBuf,
        /// Validate and report, writing nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write each closed phase's events to <dir>/P-n.jsonl, scanned
    /// first. Nothing is exported unless you run this. Not listed in
    /// help.
    #[command(hide = true)]
    Export { dir: PathBuf },
    /// Dump the log as JSON lines, from a sequence number. Not listed
    /// in help: `ris watch` serves clients.
    #[command(hide = true)]
    Log {
        #[arg(long, default_value_t = 1)]
        from: u64,
    },
    /// Follow the log: wait for new events and print each as a JSON
    /// line as it is appended, verified against the chain. For a GUI or
    /// daemon; runs until stopped.
    Watch {
        /// Start from this sequence number instead of the next event.
        #[arg(long)]
        from: Option<u64>,
        /// Exit after the first batch of events.
        #[arg(long)]
        once: bool,
        /// How often to look at the log's head, in milliseconds.
        #[arg(long, default_value_t = 200)]
        interval_ms: u64,
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
    /// Replace an item nobody has started with another (the person's
    /// only). Final; what depended on it waits on the replacement.
    Supersede {
        item: ItemId,
        #[arg(long)]
        by: ItemId,
    },
    /// Bring an item nobody has started from the backlog into the open
    /// phase, so a seat may claim it (the person's only).
    BringIn {
        item: ItemId,
    },
}

#[derive(Subcommand)]
enum ActionCmd {
    /// Raise something only the person can do, linked to the items it
    /// blocks (they wait) or informs (they proceed). The person's only:
    /// a blocked agent raises an item or a decision instead.
    Raise {
        /// What to do, in a sentence.
        what: String,
        /// How to tell it is done.
        #[arg(long)]
        done_when: String,
        /// An item that waits for it. Repeatable.
        #[arg(long = "blocks")]
        blocks: Vec<ItemId>,
        /// An item that proceeds and is shaped by it. Repeatable.
        #[arg(long = "informs")]
        informs: Vec<ItemId>,
        /// A shell command that passes once it is done; the person's
        /// inbox, summary and up run it and close the action. The person's only.
        #[arg(long)]
        check: Option<String>,
    },
    /// Give an open action a check: a shell command that passes once it
    /// is done. The person's only.
    Check {
        action: ris_core::ids::ActionId,
        #[arg(long)]
        command: String,
    },
    /// Record a person action done, with evidence or a note. The
    /// person's only.
    Done {
        action: ris_core::ids::ActionId,
        /// A file to attach as evidence. Repeatable.
        #[arg(long = "evidence")]
        evidence: Vec<PathBuf>,
        /// What happened, in a sentence.
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
enum SeatCmd {
    /// Add a seat: a durable pseudonym, never a real person's name.
    Add {
        name: String,
        #[arg(long, value_parser = ["lead", "builder", "auditor", "advisor"])]
        role: String,
    },
    /// Start a seat's agent unattended through the runner, in its own
    /// directory, and wait for it to stop. The person's only.
    Start {
        seat: String,
        /// Copy the agent's output to its log only, not to the screen.
        #[arg(long)]
        quiet: bool,
    },
    /// Record that a seat's agent stopped when its process is gone
    /// without ris seeing it end.
    Stopped { seat: String },
    /// Pause a seat, or every seat: each finishes what it holds and
    /// takes nothing new until resumed. The person's only.
    Pause(SeatSet),
    /// Resume a paused seat, or every seat.
    Resume(SeatSet),
    /// Report a fault in your seat's setup: a path your sandbox will not
    /// let you write, a host you cannot reach, a command you may not
    /// run. Shown by `ris status` until your seat's next run; not a
    /// decision. An agent's only.
    Fault {
        /// What is missing, and the command or path that needed it.
        what: String,
    },
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct SeatSet {
    /// The seat, by name.
    seat: Option<String>,
    /// Every seat in .osiris/config.toml.
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
struct ItemAdd {
    goal: String,
    /// A check from .osiris/checks.toml the item must pass. Repeatable;
    /// with none named, every declared check.
    #[arg(long = "check")]
    checks: Vec<String>,
    /// No checks: the item is done on attached evidence
    /// (`ris done --evidence`), for work a check cannot judge.
    #[arg(long, conflicts_with = "checks")]
    no_checks: bool,
    /// An item this one cannot start before. Repeatable.
    #[arg(long = "depends")]
    depends_on: Vec<ItemId>,
}

#[derive(Subcommand)]
enum DecisionCmd {
    Raise(DecisionRaise),
    /// Attach the full case to an open decision, for `ris eli5`: what it
    /// is about, pros and cons per option, and why the recommendation
    /// holds. Its raiser or the person; again to replace it.
    Explain(DecisionExplain),
}

#[derive(Args)]
struct DecisionExplain {
    decision: DecisionId,
    /// What is being decided and what it touches, in plain words.
    #[arg(long)]
    summary: String,
    /// A point for an option, as "<option number>: <text>". Repeatable.
    #[arg(long = "pro")]
    pros: Vec<String>,
    /// A point against an option, as "<option number>: <text>". Repeatable.
    #[arg(long = "con")]
    cons: Vec<String>,
    /// Why the recommended option is the recommendation.
    #[arg(long)]
    why: String,
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
enum Level {
    Low,
    Medium,
    High,
}

impl From<Level> for ris_core::primitives::RiskLevel {
    fn from(l: Level) -> Self {
        match l {
            Level::Low => Self::Low,
            Level::Medium => Self::Medium,
            Level::High => Self::High,
        }
    }
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

/// Print a line to stdout. A reader that has gone (`ris log | head`)
/// ends the program quietly rather than with a panic.
fn emit(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if let Err(e) = writeln!(out, "{line}").and_then(|()| out.flush()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("ris: writing output: {e}");
        std::process::exit(1);
    }
}

/// A failure in the `--json` envelope: its kind, the message, the next
/// step when there is one, and the details a client can act on.
fn error_envelope(e: &CliError) -> serde_json::Value {
    let text = e.to_string();
    let (mut message, mut next) = (Vec::new(), Vec::new());
    for line in text.lines() {
        match line.trim_start().strip_prefix("next:") {
            Some(n) => next.push(n.trim().to_owned()),
            None => message.push(line),
        }
    }
    use ris::schema::ErrorKind;
    let mut error = schema::Error {
        kind: match e {
            CliError::Refused(_) => ErrorKind::Refused,
            CliError::Leak { .. } => ErrorKind::Leak,
            CliError::RoleForbids { .. } => ErrorKind::RoleForbids,
            CliError::CheckFailed { .. } => ErrorKind::CheckFailed,
            CliError::ChecksFailed { .. } => ErrorKind::ChecksFailed,
            CliError::NotAProject(_) => ErrorKind::NotAProject,
            CliError::AlreadyAProject(_) => ErrorKind::AlreadyAProject,
            CliError::Config(_) => ErrorKind::Config,
            CliError::UnknownAgent(_) => ErrorKind::UnknownAgent,
            CliError::NoActor(_) => ErrorKind::NoActor,
            CliError::Contended => ErrorKind::Contended,
            CliError::Store(_) => ErrorKind::Store,
            CliError::Io(_) => ErrorKind::Io,
            CliError::Other(_) => ErrorKind::Other,
        },
        message: message.join("\n"),
        next: (!next.is_empty()).then(|| next.join("; ")),
        refusal: None,
        scanned: None,
        hits: None,
        item: None,
        stage: None,
        failed: None,
        seat: None,
        role: None,
    };
    match e {
        CliError::Refused(r) => {
            error.next = Some(r.next_step());
            error.refusal = Some(r.clone());
        }
        CliError::Leak { what, hits } => {
            error.scanned = Some(what.to_string());
            error.hits = Some(
                hits.iter()
                    .map(|h| schema::Hit {
                        file: h.file.clone(),
                        line: h.line,
                        category: h.category.description().to_owned(),
                    })
                    .collect(),
            );
        }
        CliError::ChecksFailed {
            item,
            stage,
            failed,
            ..
        } => {
            error.item = Some(item.to_string());
            error.stage = Some(stage.to_string());
            error.failed = Some(
                failed
                    .iter()
                    .map(|(check, evidence)| schema::FailedCheck {
                        check: check.to_string(),
                        evidence: evidence.to_string(),
                    })
                    .collect(),
            );
        }
        CliError::RoleForbids { seat, role, .. } => {
            error.seat = Some(seat.clone());
            error.role = Some(*role);
        }
        _ => {}
    }
    json!({ "format": render::JSON_FORMAT, "ok": false, "error": error })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    ris::style::init(cli.json);
    match run(&cli) {
        Ok(out) => {
            match (cli.json, out.stream) {
                (true, true) => {
                    for item in out.json.as_array().into_iter().flatten() {
                        emit(&render::envelope(item).to_string());
                    }
                }
                (true, false) => {
                    emit(&serde_json::to_string_pretty(&render::envelope(&out.json)).unwrap())
                }
                (false, _) if out.text.is_empty() => {}
                (false, _) => emit(&out.text),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if cli.json {
                // Failures go to stdout too, so a client reads one
                // stream; the exit status still says what kind it was.
                emit(&serde_json::to_string_pretty(&error_envelope(&e)).unwrap());
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
                CliError::Refused(_)
                | CliError::Leak { .. }
                | CliError::RoleForbids { .. }
                | CliError::NoActor(_) => 2,
                CliError::ChecksFailed { .. } | CliError::CheckFailed { .. } => 3,
                _ => 1,
            })
        }
    }
}

/// Item ids as a line of text reads them: `I-3, I-7`.
fn id_list(items: &[ItemId]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
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
        return Ok(Output::of(
            text,
            &schema::Init {
                root: p.root.to_string_lossy().into_owned(),
                local,
            },
        ));
    }
    if let Cmd::Rules = cli.command {
        return Ok(Output::of(
            ris::project::RULES.trim_end(),
            &schema::Rules {
                rules: ris::project::RULES.to_owned(),
            },
        ));
    }
    if let Cmd::Schema = cli.command {
        // The contract is the program's, not a project's: it prints
        // anywhere, for anyone.
        return Ok(Output::of(schema::text().trim_end(), &schema::document()));
    }
    let mut p = Project::find(&cwd)?;
    let actor = p.actor()?;
    forbidden_by_role(&p, &actor, &cli.command)?;
    // D-48: the person's ris catches up with the last merge before it
    // does anything else. A seat's never does: it runs in a sandbox.
    if actor == Actor::Person
        && !matches!(cli.command, Cmd::Install)
        && let Some(cfg) = &p.config.install
    {
        let l = p.load()?;
        if let Some(line) = ris::install::catch_up(&p.root, cfg, &l.log) {
            eprintln!("{line}");
        }
    }

    match &cli.command {
        Cmd::Init { .. } | Cmd::Rules | Cmd::Schema => unreachable!(),

        Cmd::Worktree { seat } => {
            let path = make_worktree(&p, seat)?;
            Ok(Output::of(
                format!(
                    "worktree for {seat} at {}\n  next: cd there and start the agent with RIS_AGENT={seat}",
                    path.display()
                ),
                &schema::Worktree {
                    seat: seat.clone(),
                    path: path.to_string_lossy().into_owned(),
                },
            ))
        }

        Cmd::Up { seats } => up(&mut p, &actor, seats),

        Cmd::Install => install(&p, &actor),

        Cmd::Down { seats } => down(&mut p, &actor, seats),

        Cmd::Logs { seat, follow } => match seat {
            Some(seat) => logs(&p, seat, *follow, cli.json),
            None => logs_all(&p, *follow, cli.json),
        },

        Cmd::Seat {
            action: SeatCmd::Add { name, role },
        } => {
            if actor != Actor::Person {
                return Err(CliError::Other(
                    "seats are the person's to add; run ris seat add with RIS_AGENT=person".into(),
                ));
            }
            let role = ris::config::Role::parse(role).expect("clap admits only role names");
            let l = p.load()?;
            let retired = names_in_log(&l.state);
            let path = p.root.join(ris::project::DIR).join("config.toml");
            let text = std::fs::read_to_string(&path)?;
            let out = ris::config::Config::with_seat(&text, name, role, &retired)
                .map_err(|e| CliError::Other(format!("refused: {e}")))?;
            std::fs::write(&path, out)?;
            let next = if !role.writes() {
                format!("start it in the main checkout with RIS_AGENT={name}")
            } else {
                format!("`ris worktree {name}`, then start it there with RIS_AGENT={name}")
            };
            Ok(Output::of(
                format!("seat {name} added\n  next: {next}"),
                &schema::SeatAdded {
                    seat: name.clone(),
                    role,
                },
            ))
        }

        Cmd::Seat {
            action: SeatCmd::Start { seat, quiet },
        } => start_seat(&mut p, &actor, seat, *quiet || cli.json),

        Cmd::Seat {
            action: SeatCmd::Stopped { seat },
        } => {
            p.run(&Command::RunStop {
                seat: AgentName(seat.clone()),
                actor: actor.clone(),
                status: None,
            })?;
            Ok(Output::of(
                format!("{seat} recorded as stopped"),
                &schema::SeatRef { seat: seat.clone() },
            ))
        }

        Cmd::Seat {
            action: action @ (SeatCmd::Pause(set) | SeatCmd::Resume(set)),
        } => {
            let pause = matches!(action, SeatCmd::Pause(_));
            let seats: Vec<AgentName> = match &set.seat {
                Some(name) if p.config.agents.contains_key(name) => vec![AgentName(name.clone())],
                Some(name) => return Err(CliError::UnknownAgent(name.clone())),
                None => p.config.agents.keys().cloned().map(AgentName).collect(),
            };
            let (l, _) = p.run(&if pause {
                Command::Pause {
                    seats: seats.clone(),
                    actor: actor.clone(),
                }
            } else {
                Command::Resume {
                    seats: seats.clone(),
                    actor: actor.clone(),
                }
            })?;
            let names: Vec<String> = seats.iter().map(ToString::to_string).collect();
            let holding: Vec<String> = l
                .state
                .items
                .values()
                .filter_map(|i| match &i.state {
                    ris_core::primitives::ItemState::Claimed { agent }
                        if pause && seats.contains(agent) =>
                    {
                        Some(format!("{agent} finishes {}", i.id))
                    }
                    _ => None,
                })
                .collect();
            let verb = if pause { "paused" } else { "resumed" };
            let mut text = format!("{verb}: {}", names.join(", "));
            if !holding.is_empty() {
                text.push_str(&format!("\n  {}", holding.join("; ")));
            }
            Ok(Output::of(
                text,
                &schema::SeatsSet {
                    paused: pause.then(|| names.clone()),
                    resumed: (!pause).then(|| names.clone()),
                    finishing: holding,
                },
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
                let phase = l.state.open_phase().expect("just opened").id;
                let mut text = match &branch {
                    Some(b) => format!("opened {phase} on branch {b}"),
                    None => format!("opened {phase}"),
                };
                // The standing rule: every phase ends with the docs
                // describing what it built.
                let mut docs = None;
                if p.config.docs_item {
                    let (l, _) = p.run(&Command::ItemAdd {
                        goal: p.text("goal", DOCS_GOAL)?,
                        checks: p.checks.for_item(None),
                        depends_on: Vec::new(),
                    })?;
                    let id = *l.state.items.keys().last().expect("just added");
                    text.push_str(&format!(
                        "\n  {id} brings the docs up to date; close waits for it"
                    ));
                    docs = Some(id);
                }
                // Said now, so the configuration is fixed before `ris
                // up`, which refuses to start seats until it is.
                let missing = preflight(&p);
                if !missing.is_empty() {
                    text.push('\n');
                    text.push_str(&ris::preflight::render(&missing));
                }
                Ok(Output::of(
                    text,
                    &schema::PhaseOpened {
                        phase,
                        branch,
                        docs_item: docs,
                        preflight: missing,
                    },
                ))
            }
            PhaseCmd::Close => {
                let l = p.load()?;
                let phase = l
                    .state
                    .open_phase()
                    .ok_or(CliError::Refused(ris_core::Refusal::NoOpenPhase))?;
                let (phase_id, goal) = (phase.id, l.text(phase.at, "goal").to_owned());
                // Facts become final here: check the derived index
                // against a scan before anything is written.
                l.state
                    .check_index()
                    .map_err(|e| CliError::Other(e.to_string()))?;
                // The docs item, if this phase has one, is merged (or
                // superseded) before the phase closes.
                if let Some(docs) = l.state.items.values().find(|i| {
                    i.phase == phase_id
                        && l.text(i.at, "goal") == DOCS_GOAL
                        && !matches!(
                            i.state,
                            ris_core::primitives::ItemState::Merged { .. }
                                | ris_core::primitives::ItemState::Superseded { .. }
                        )
                }) {
                    return Err(CliError::Other(format!(
                        "refused: {} (the docs) is not merged, so the docs may not describe what this phase built\n  \
                         next: bring them up to date and merge {}, or supersede it",
                        docs.id, docs.id
                    )));
                }
                let closing = closing_merge(&mut p, phase_id, &goal)?;
                let (l, events) = p.run(&Command::PhaseClose)?;
                let closed = events.last().expect("close emits one event");
                let report = match &closed.structure {
                    ris_core::event::Structure::PhaseClosed { report, .. } => *report,
                    _ => unreachable!(),
                };
                let (text, phase) = render::report_data(&l);
                Ok(Output::of(
                    format!(
                        "{text}\n{closing}\nthe change set as a map of the source: `ris map --html <file>`"
                    ),
                    &schema::PhaseClosed {
                        report,
                        phase,
                        closing_merge: closing,
                    },
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
                    checks: p.checks.for_item(if a.no_checks {
                        Some(&[])
                    } else if a.checks.is_empty() {
                        None
                    } else {
                        Some(&a.checks)
                    }),
                    depends_on: a.depends_on.clone(),
                })?;
                let id = *l.state.items.keys().last().expect("just added");
                Ok(Output::of(
                    format!("added {id}"),
                    &schema::ItemRef { item: id },
                ))
            }
            ItemCmd::Supersede { item, by } => {
                p.run(&Command::Supersede {
                    item: *item,
                    by: *by,
                    actor: actor.clone(),
                })?;
                Ok(Output::of(
                    format!("{item} superseded by {by}"),
                    &schema::Superseded {
                        item: *item,
                        superseded_by: *by,
                    },
                ))
            }
            ItemCmd::BringIn { item } => {
                let (l, _) = p.run(&Command::BringIn {
                    item: *item,
                    actor: actor.clone(),
                })?;
                let phase = l.state.items[item].phase;
                Ok(Output::of(
                    format!("{item} brought into {phase}"),
                    &schema::BroughtIn { item: *item, phase },
                ))
            }
        },

        Cmd::Ready => Ok(render::ready(&p.load()?, &actor)),

        Cmd::Claim { item } => {
            let agent = agent_only(&actor)?;
            let (l, _) = p.run(&Command::Claim {
                item: *item,
                agent: agent.clone(),
            })?;
            let role = p
                .config
                .agents
                .get(&agent.0)
                .map(|a| a.role.to_string())
                .unwrap_or_default();
            let rules_path = p.rules_path();
            let found = std::fs::read_to_string(&rules_path).ok();
            let shown = rules_path
                .strip_prefix(&p.root)
                .unwrap_or(&rules_path)
                .display()
                .to_string();
            let content = found.as_deref().unwrap_or(ris::project::RULES);
            let (text, data) = ris::brief::brief(
                &l,
                *item,
                &role,
                &ris::brief::Rules {
                    path: found.as_ref().map(|_| shown.as_str()),
                    content,
                },
            );
            Ok(Output::of(
                format!(
                    "claimed {item}
{text}"
                ),
                &data,
            ))
        }

        Cmd::Eli5 { decision } => {
            let l = p.load()?;
            let d = l.state.decisions.get(decision).ok_or(CliError::Refused(
                ris_core::Refusal::UnknownDecision(*decision),
            ))?;
            Ok(render::eli5(&l, d))
        }

        Cmd::Decision { action } => {
            match action {
                DecisionCmd::Explain(e) => {
                    let points = |field: &str, list: &[String]| -> Result<Vec<(usize, ris_core::text::Text)>, CliError> {
                    list.iter()
                        .map(|raw| {
                            let (n, text) = raw
                                .split_once(':')
                                .and_then(|(n, t)| Some((n.trim().parse::<usize>().ok()?, t.trim())))
                                .ok_or_else(|| {
                                    CliError::Other(format!(
                                        "--{field} takes \"<option number>: <text>\", as in --{field} \"0: cheaper to reverse\""
                                    ))
                                })?;
                            Ok((n, p.text(&format!("option.{n}.{field}"), text)?))
                        })
                        .collect()
                };
                    let pros = points("pro", &e.pros)?;
                    let cons = points("con", &e.cons)?;
                    let summary = p.text("summary", &e.summary)?;
                    let why = p.text("why", &e.why)?;
                    p.run(&Command::Explain {
                        decision: e.decision,
                        by: actor.clone(),
                        summary,
                        pros,
                        cons,
                        why,
                    })?;
                    Ok(Output::of(
                        format!(
                            "{} explained\n  next: `ris eli5 {}` shows it",
                            e.decision, e.decision
                        ),
                        &schema::DecisionRef {
                            decision: e.decision,
                        },
                    ))
                }
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
                    let id = *l.state.decisions.keys().last().expect("just raised");
                    let text = match class {
                        DecisionClass::Mechanical => format!("{id} taken: option {}", d.recommend),
                        DecisionClass::Substantive => format!(
                            "{id} queued for the person's review{}",
                            d.about
                                .map(|i| format!("; {i} is set aside, take another"))
                                .unwrap_or_default()
                        ),
                    };
                    Ok(Output::of(text, &schema::DecisionRef { decision: id }))
                }
            }
        }

        Cmd::Decide {
            decision,
            choose,
            other,
            rationale,
        } => {
            let Some(decision) = decision else {
                return if cli.json {
                    Ok(ris::decide::queue_output(&p.load()?))
                } else {
                    ris::decide::walk_stdin(&mut p, &actor)
                };
            };
            let rationale = p.text("rationale", rationale)?;
            let chosen = match (choose, other) {
                (Some(n), None) => Choice::Option(*n),
                (None, Some(answer)) => Choice::Other(p.text("other", answer)?),
                (None, None) => {
                    return Err(CliError::Other(format!(
                        "ris decide {decision} needs --choose <option number> or --other \"your own answer\"; `ris decide` alone walks the queue"
                    )));
                }
                (Some(_), Some(_)) => unreachable!("clap refuses --choose with --other"),
            };
            let (l, events) = p.run(&Command::Decide {
                decision: *decision,
                by: actor.clone(),
                chosen,
                rationale,
            })?;
            let unblocked: Vec<ItemId> = events
                .iter()
                .filter_map(|e| match &e.structure {
                    ris_core::event::Structure::ItemReady { item } => Some(*item),
                    _ => None,
                })
                .collect();
            let _ = l;
            let picked = match choose {
                Some(n) => format!("option {n}"),
                None => "your own answer, recorded as a new option".to_owned(),
            };
            Ok(Output::of(
                format!(
                    "{decision} taken: {picked}{}",
                    if unblocked.is_empty() {
                        String::new()
                    } else {
                        format!("; ready again: {}", id_list(&unblocked))
                    }
                ),
                &schema::Decide::Taken {
                    decision: *decision,
                    unblocked,
                },
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
            Ok(Output::of(
                text,
                &schema::Done {
                    item: *item,
                    evidence: ids,
                },
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
                if let Some(phase) = ris::status::phase_branch(&p, &l) {
                    let here =
                        git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
                    if here != phase {
                        return Err(CliError::Other(format!(
                            "refused: the main checkout is not on {phase}, so a commit here is not on the phase\n  \
                             next: work in a seat's worktree (`ris up` makes one for each writing seat) and merge from there"
                        )));
                    }
                }
                // Width one: the commit is already on the phase branch.
                let commit = match commit {
                    Some(c) => c.clone(),
                    None => checks::git_head(&p.work)?,
                };
                require_one_commit(&p, &l, *item, &commit)?;
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
            Ok(Output::of(
                text,
                &schema::Merged {
                    item: *item,
                    commit,
                },
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
                results.push(schema::CheckResult {
                    check: name.clone(),
                    passed,
                });
            }
            if failed {
                return Err(CliError::CheckFailed { lines });
            }
            Ok(Output::of(
                lines.join("\n"),
                &schema::Checks { checks: results },
            ))
        }

        Cmd::Audit {
            item: None,
            all,
            file: None,
            ..
        } => {
            let l = p.load()?;
            let scope = render::Scope::of(&l, *all);
            Ok(render::audit_queue(&l, &actor, scope))
        }

        Cmd::Audit {
            item: Some(item), ..
        } => {
            let by = agent_only(&actor)?;
            p.run(&Command::Audit {
                item: *item,
                by: by.clone(),
            })?;
            Ok(Output::of(
                format!("{item} reviewed by {by}"),
                &schema::Audit::Recorded(schema::Audited { item: *item, by }),
            ))
        }

        Cmd::Release { item } => {
            p.run(&Command::Release {
                item: *item,
                by: actor.clone(),
            })?;
            Ok(Output::of(
                format!("released {item}; it is ready again"),
                &schema::ItemRef { item: *item },
            ))
        }

        Cmd::Inbox => {
            let closed = close_checked(&mut p, &actor)?;
            Ok(render::inbox(&p.load()?, &actor, closed))
        }

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
            Ok(Output::of(
                format!("sent {id} to {to}"),
                &schema::Sent { message: id },
            ))
        }

        Cmd::Seat {
            action: SeatCmd::Fault { what },
        } => {
            let seat = agent_only(&actor)?;
            let what = p.text("what", what)?;
            p.run(&Command::SetupFault {
                seat: seat.clone(),
                what,
            })?;
            Ok(Output::of(
                format!(
                    "{seat}: setup fault recorded; `ris status` shows it to the person\n  \
                     next: take another item that does not need it, or stop if none can be done"
                ),
                &schema::SeatRef { seat: seat.0 },
            ))
        }

        Cmd::Show { id } => {
            // Evidence: E-<hex> or E<hex>, the full hash or a unique
            // prefix of at least 8 characters. The stored output is
            // printed as it is.
            let hex = id
                .strip_prefix(['E', 'e'])
                .map(|r| r.strip_prefix('-').unwrap_or(r))
                .filter(|h| h.len() >= 8 && h.chars().all(|c| c.is_ascii_hexdigit()));
            match hex {
                Some(hex) => show_evidence(&p, &hex.to_ascii_lowercase()),
                None => render::show(&p.load()?, id).map_err(CliError::Other),
            }
        }

        Cmd::Action {
            action:
                ActionCmd::Raise {
                    what,
                    done_when,
                    blocks,
                    informs,
                    check,
                },
        } => {
            // An action is the person's own to-do; the core refuses an
            // agent too, but refusing here first scans no text for it.
            if actor != Actor::Person {
                return Err(CliError::Refused(
                    ris_core::Refusal::RaiseActionIsForThePerson,
                ));
            }
            let check = check.as_deref().map(|c| p.text("check", c)).transpose()?;
            let (l, events) = p.run(&Command::RaiseAction {
                by: actor.clone(),
                what: p.text("what", what)?,
                done_when: p.text("done_when", done_when)?,
                blocks: blocks.clone(),
                informs: informs.clone(),
            })?;
            let id = *l.state.actions.keys().last().expect("just raised");
            if let Some(check) = check {
                p.run(&Command::SetActionCheck {
                    action: id,
                    by: actor.clone(),
                    check,
                })?;
            }
            let blocked: Vec<String> = events
                .iter()
                .filter_map(|e| match &e.structure {
                    ris_core::event::Structure::ItemBlocked { item, .. } => Some(item.to_string()),
                    _ => None,
                })
                .collect();
            let mut text = format!("{id} raised for the person");
            if !blocked.is_empty() {
                text.push_str(&format!(
                    "; {} set aside until it is done",
                    blocked.join(", ")
                ));
            }
            Ok(Output::of(
                text,
                &schema::ActionRaised {
                    action: id,
                    blocked,
                },
            ))
        }

        Cmd::Action {
            action:
                ActionCmd::Done {
                    action,
                    evidence,
                    note,
                },
        } => {
            let note = note.as_deref().map(|n| p.text("note", n)).transpose()?;
            let mut ids = Vec::new();
            for file in evidence {
                ids.push(p.evidence.put_file(file)?);
            }
            let (_, events) = p.run(&Command::ActionDone {
                action: *action,
                by: actor.clone(),
                evidence: ids,
                note,
            })?;
            let ready: Vec<ItemId> = events
                .iter()
                .filter_map(|e| match &e.structure {
                    ris_core::event::Structure::ItemReady { item } => Some(*item),
                    _ => None,
                })
                .collect();
            let mut text = format!("{action} done");
            if !ready.is_empty() {
                text.push_str(&format!("; ready again: {}", id_list(&ready)));
            }
            Ok(Output::of(
                text,
                &schema::ActionDone {
                    action: *action,
                    ready,
                },
            ))
        }

        Cmd::Action {
            action: ActionCmd::Check { action, command },
        } => {
            p.run(&Command::SetActionCheck {
                action: *action,
                by: actor.clone(),
                check: p.text("check", command)?,
            })?;
            let closed = close_checked(&mut p, &actor)?;
            Ok(Output::of(
                if closed.is_empty() {
                    format!("{action} has a check now")
                } else {
                    closed.join("\n")
                },
                &schema::ActionChecked {
                    action: *action,
                    checks: closed,
                },
            ))
        }

        Cmd::Summary { all } => {
            let closed = close_checked(&mut p, &actor)?;
            Ok(render::summary(&p.load()?, &actor, *all, closed))
        }

        Cmd::Report => Ok(render::report(&p.load()?)),

        Cmd::Map { phase, html } => {
            let l = p.load()?;
            let range = ris::map::range(&p, &l, *phase)?;
            ris::map::output(ris::map::build(&p, &l, &range)?, html.as_deref())
        }

        Cmd::Audit {
            file: Some(path),
            risk,
            why,
            ..
        } => {
            let (Some(level), Some(why)) = (risk, why) else {
                return Err(CliError::Other(
                    "a risk judgement needs a level and a reason\n  \
                     next: `ris audit --file <path> --risk <low|medium|high> --why \"<one line>\"`"
                        .into(),
                ));
            };
            let by = agent_only(&actor)?;
            let l = p.load()?;
            let open = l
                .state
                .open_phase()
                .ok_or(ris_core::refusal::Refusal::NoOpenPhase)?
                .id;
            // The judgement is of a change, so the file must be one the
            // open phase changed and the map shows.
            let range = ris::map::range(&p, &l, Some(open))?;
            let map = ris::map::survey(&p, &l, &range)?;
            if !map
                .nodes
                .iter()
                .any(|n| n.path == *path && n.status != "unchanged")
            {
                return Err(CliError::Other(format!(
                    "{open} has no changed file on the map at that path\n  \
                     next: `ris map` lists the changed files; give the path as it is written there"
                )));
            }
            let level: ris_core::primitives::RiskLevel = (*level).into();
            p.run(&Command::JudgeRisk {
                by: by.clone(),
                level,
                path: p.text("path", path)?,
                why: p.text("why", why)?,
            })?;
            let word = ris::map::level_word(level);
            Ok(Output::of(
                format!("{path} judged {word} by {by}; `ris map` shows it to the person"),
                &schema::Audit::Risk(schema::RiskRecorded {
                    phase: open,
                    path: path.clone(),
                    level,
                    seat: by.0,
                }),
            ))
        }

        Cmd::Status => Ok(ris::status::render(&p, &p.load()?)),

        Cmd::Scan { paths } => {
            let files = ris::exits::files_to_scan(&p.work, paths)?;
            if files.is_empty() {
                return Err(CliError::Other(
                    "nothing scanned: no files matched, so there is nothing to call clean\n  \
                     next: name the files or directories to scan"
                        .into(),
                ));
            }
            let hits = ris::exits::scan_files(&p.work, &files, &p.policy)?;
            if !hits.is_empty() {
                return Err(CliError::Leak {
                    what: "the working tree".into(),
                    hits,
                });
            }
            Ok(Output::of(
                format!("clean: {} file(s) scanned", files.len()),
                &schema::Scanned {
                    files: files.len(),
                    hits: Vec::new(),
                },
            ))
        }

        Cmd::Import { file, dry_run } => {
            if actor != Actor::Person {
                return Err(CliError::Other(
                    "imported decisions are the person's; run ris import with RIS_AGENT=person"
                        .into(),
                ));
            }
            let l = p.load()?;
            if l.state.open_phase().is_none() {
                return Err(CliError::Refused(ris_core::Refusal::NoOpenPhase));
            }
            let source = std::fs::read_to_string(file)?;
            let plan = ris::import::plan(
                &source,
                &l.state,
                &ris::import::Recorded::from(&l),
                &p.checks,
                &p.policy,
            )
            .map_err(|problems| {
                CliError::Other(format!(
                    "refused: nothing was imported; {} problem(s):\n  {}",
                    problems.len(),
                    problems.join("\n  ")
                ))
            })?;
            let imported = |written: bool| schema::Imported {
                items: plan.items,
                decisions: plan.decisions,
                open: plan.open,
                written,
            };
            if *dry_run {
                return Ok(Output::of(
                    format!(
                        "valid: would import {} item(s) and {} decision(s), {} of them open; nothing written",
                        plan.items, plan.decisions, plan.open
                    ),
                    &imported(false),
                ));
            }
            let first_item = l.state.next_item_id().0;
            let first_decision = l.state.next_decision_id().0;
            for c in &plan.commands {
                p.run(c)?;
            }
            let range = |prefix: &str, first: u32, n: usize| match n {
                0 => "none".to_owned(),
                1 => format!("{prefix}-{first}"),
                _ => format!("{prefix}-{first} to {prefix}-{}", first + n as u32 - 1),
            };
            Ok(Output::of(
                format!(
                    "imported items {} and decisions {}{}",
                    range("I", first_item, plan.items),
                    range("D", first_decision, plan.decisions),
                    match plan.open {
                        0 => String::new(),
                        n => format!("; {n} open, waiting in ris inbox"),
                    }
                ),
                &imported(true),
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
            Ok(Output::of(
                format!(
                    "exported {} closed phase(s) to {} ({new} new)",
                    files.len(),
                    dir.display()
                ),
                &schema::Exported {
                    phases: files.iter().map(|f| f.name.clone()).collect(),
                    new,
                },
            ))
        }

        Cmd::Verify => {
            let l = p.load()?;
            let v =
                ris_core::event::verify(&l.log).map_err(ris_core::store::StoreError::Corrupt)?;
            l.state
                .check_index()
                .map_err(|e| CliError::Other(e.to_string()))?;
            Ok(Output::of(
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
                &schema::Verified {
                    len: v.len,
                    head: v.head,
                    missing_text: v.missing_text,
                },
            ))
        }

        Cmd::Watch {
            from,
            once,
            interval_ms,
        } => {
            // Verify what is there once; after that only what is new.
            let l = p.load()?;
            let start = from.unwrap_or(l.state.len + 1).max(1);
            let (mut len, mut head) = match start - 1 {
                0 => (0, ris_core::ids::Hash::ZERO),
                n => (
                    n,
                    l.log.get(n as usize - 1).map(|e| e.hash).ok_or_else(|| {
                        CliError::Other(format!(
                            "--from {start} is beyond the log's {} events",
                            l.state.len
                        ))
                    })?,
                ),
            };
            drop(l);
            let interval = std::time::Duration::from_millis((*interval_ms).max(10));
            loop {
                let (stored, _) = p.head()?;
                if stored < len {
                    return Err(CliError::Other(format!(
                        "the log shrank from {len} to {stored} events; it was replaced, not appended to"
                    )));
                }
                if stored > len {
                    let new = p.read_from(len + 1)?;
                    let v = ris_core::event::verify_after(len, head, &new)
                        .map_err(ris_core::store::StoreError::Corrupt)?;
                    let lines: Vec<String> = new
                        .iter()
                        .map(|e| serde_json::to_string(e).unwrap())
                        .collect();
                    (len, head) = (v.len, v.head);
                    if *once {
                        return Ok(Output::stream(lines.join("\n"), &new));
                    }
                    for (line, e) in lines.iter().zip(&new) {
                        if cli.json {
                            emit(&render::envelope(e).to_string());
                        } else {
                            emit(line);
                        }
                    }
                }
                std::thread::sleep(interval);
            }
        }

        Cmd::Log { from } => {
            let l = p.load()?;
            let events: Vec<_> = l.log.iter().filter(|e| e.seq >= *from).collect();
            let lines: Vec<String> = events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap())
                .collect();
            Ok(Output::stream(lines.join("\n"), &events))
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

/// Refuse, before git is asked to commit in `dir`, when git has no
/// identity to commit with there. `git var GIT_COMMITTER_IDENT` fails
/// exactly when a commit would, so the person is told what is missing
/// and how to set it, instead of reading the failed merge or rebase as
/// a conflict. `what` names the step that commits.
fn require_git_identity(dir: &std::path::Path, what: &str) -> Result<(), CliError> {
    match git_identity_missing(dir, what) {
        None => Ok(()),
        Some(why) => Err(CliError::Other(format!(
            "refused: {why}, then run it again"
        ))),
    }
}

/// Why git cannot commit in `dir` for `what`, with the commands that
/// set an identity, or `None` when it can.
fn git_identity_missing(dir: &std::path::Path, what: &str) -> Option<String> {
    git(dir, &["var", "GIT_COMMITTER_IDENT"]).err().map(|_| {
        format!(
            "git has no identity to commit with here, and {what} makes a commit\n  \
             next: `git config user.name \"<your name>\"` and `git config user.email \"<your address>\"` \
             (add --global to set them for every repository)"
        )
    })
}

/// `ris show E-<hex>`: one piece of evidence, by full hash or a unique
/// prefix. Content-addressed, so the bytes are checked against the id.
fn show_evidence(p: &Project, hex: &str) -> Result<Output, CliError> {
    let dir = p.root.join(ris::project::DIR).join("evidence");
    let matches: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(hex))
        .collect();
    let name = match matches.as_slice() {
        [one] => one,
        [] => return Err(CliError::Other(format!("no evidence E-{hex}"))),
        many => {
            return Err(CliError::Other(format!(
                "E-{hex} matches {} pieces of evidence; give more of the hash",
                many.len()
            )));
        }
    };
    let bytes = std::fs::read(dir.join(name))?;
    if ris::evidence::EvidenceStore::id_of(&bytes).to_string() != format!("E-{name}") {
        return Err(CliError::Other(format!(
            "evidence E-{name} does not match its hash; it was changed after it was stored"
        )));
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Output::of(
        text.trim_end().to_owned(),
        &schema::Show::Evidence(schema::EvidenceContent {
            evidence: format!("E-{name}"),
            content: text.clone(),
        }),
    ))
}

/// Every agent name the log has seen: claimants, authors, auditors,
/// deciders and message parties. A name in the log is never reused.
fn names_in_log(state: &ris_core::state::State) -> Vec<String> {
    use ris_core::primitives::{DecisionState, ItemState};
    let mut actors: Vec<&Actor> = Vec::new();
    for d in state.decisions.values() {
        actors.push(&d.by);
        if let DecisionState::Taken { by, .. } = &d.state {
            actors.push(by);
        }
    }
    actors.extend(state.messages.iter().map(|m| &m.from));
    let mut names: std::collections::BTreeSet<String> = actors
        .into_iter()
        .filter_map(|a| match a {
            Actor::Agent(n) => Some(n.0.clone()),
            Actor::Person => None,
        })
        .collect();
    names.extend(state.messages.iter().map(|m| m.to.0.clone()));
    for i in state.items.values() {
        names.extend(i.author.iter().map(|a| a.0.clone()));
        names.extend(i.audited_by.iter().map(|a| a.0.clone()));
        if let ItemState::Claimed { agent } = &i.state {
            names.insert(agent.0.clone());
        }
    }
    names.into_iter().collect()
}

/// What a seat's role rules out, checked before anything runs. An
/// auditor reviews and never writes to the tree, so it cannot take,
/// finish or merge an item, and has no worktree. An advisor works
/// beside the person: it reads, adds items, sends messages and raises
/// decisions for the person, and is refused everything else, by a list
/// of what it may do rather than of what it may not, so a command added
/// later is refused to it until it is listed. A lead and a builder may
/// do everything; the core separately refuses anyone auditing an item
/// they authored.
fn forbidden_by_role(p: &Project, actor: &Actor, command: &Cmd) -> Result<(), CliError> {
    use ris::config::Role;
    let role_of = |seat: &str| p.config.agents.get(seat).map(|a| a.role);
    let refuse = |seat: &str, role: Role, what: &'static str| {
        Err(CliError::RoleForbids {
            seat: seat.to_owned(),
            role,
            what,
        })
    };
    if let Cmd::Worktree { seat } = command
        && let Some(role) = role_of(seat)
        && !role.writes()
    {
        return refuse(
            seat,
            role,
            "need a worktree: it reads the main checkout and never writes",
        );
    }
    let Actor::Agent(name) = actor else {
        return Ok(());
    };
    // A risk judgement is the reviewer's: a seat that writes the change
    // does not grade it.
    if let (Cmd::Audit { file: Some(_), .. }, Some(role)) = (command, role_of(&name.0))
        && role != Role::Auditor
    {
        return refuse(
            &name.0,
            role,
            "record a risk judgement: an auditor does, as it reviews the phase",
        );
    }
    match role_of(&name.0) {
        Some(Role::Auditor) => match command {
            Cmd::Claim { .. } => refuse(&name.0, Role::Auditor, "claim items"),
            Cmd::Done { .. } => refuse(&name.0, Role::Auditor, "finish items"),
            Cmd::Merge { .. } => refuse(&name.0, Role::Auditor, "merge items"),
            _ => Ok(()),
        },
        Some(Role::Advisor) => match advisor_forbids(command) {
            Some(what) => refuse(&name.0, Role::Advisor, what),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// What an advisor may not do with `command`, or `None` when it may:
/// read, add an item, send a message, raise a substantive decision and
/// explain it. A mechanical decision is taken as it is raised, which is
/// deciding, so an advisor raises only substantive ones.
fn advisor_forbids(command: &Cmd) -> Option<&'static str> {
    match command {
        Cmd::Ready
        | Cmd::Inbox
        | Cmd::Show { .. }
        | Cmd::Summary { .. }
        | Cmd::Report
        | Cmd::Map { html: None, .. }
        | Cmd::Status
        | Cmd::Verify
        | Cmd::Eli5 { .. }
        | Cmd::Rules
        | Cmd::Logs { .. }
        | Cmd::Log { .. }
        | Cmd::Watch { .. }
        | Cmd::Scan { .. }
        | Cmd::Audit {
            item: None,
            file: None,
            ..
        }
        | Cmd::Send { .. }
        | Cmd::Seat {
            action: SeatCmd::Fault { .. },
        }
        | Cmd::Item {
            action: ItemCmd::Add(_),
        }
        | Cmd::Decision {
            action: DecisionCmd::Explain(_),
        } => None,
        Cmd::Decision {
            action: DecisionCmd::Raise(d),
        } => (!d.substantive)
            .then_some("take a decision: raise it with --substantive, for the person to take"),
        Cmd::Claim { .. } => Some("claim items"),
        Cmd::Done { .. } => Some("finish items"),
        Cmd::Merge { .. } => Some("merge items"),
        Cmd::Decide { .. } => Some("take decisions"),
        Cmd::Release { .. } => Some("release items"),
        Cmd::Audit { item: Some(_), .. } => Some("record reviews"),
        Cmd::Audit { file: Some(_), .. } => Some("record risk judgements"),
        Cmd::Check { .. } => Some("run checks"),
        _ => Some("change the harness"),
    }
}

/// Create a writing seat's worktree where `worktrees` in the
/// configuration puts it (by default beside the repository), on its own
/// branch, with the rules imported. Its path.
fn make_worktree(p: &Project, seat: &str) -> Result<PathBuf, CliError> {
    if !p.config.agents.contains_key(seat) {
        return Err(CliError::UnknownAgent(seat.to_owned()));
    }
    let path = p.config.worktree_path(&p.root, seat);
    if path.exists() {
        return Err(CliError::Other(format!(
            "{} already exists\n  next: start the agent there, or remove it with `git worktree remove`",
            shown_beside(p, &path)
        )));
    }
    // Inside the main checkout, the worktree would show there as
    // untracked, and the main checkout must be clean to open or close
    // a phase.
    if let Ok(inside) = path.strip_prefix(ris::config::normalize(&p.root)) {
        let pattern = format!("/{}/", inside.to_string_lossy().replace('\\', "/"));
        ris::project::exclude_locally(&p.root, &[&pattern])?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
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
    // A new seat branch starts from the phase's work, not from whatever
    // the main checkout is on.
    let base = ris::status::phase_branch(p, &p.load()?);
    let mut args: Vec<&str> = if has_branch {
        vec!["worktree", "add", "--quiet", &target, seat]
    } else {
        vec!["worktree", "add", "--quiet", "-b", seat, &target]
    };
    if let (false, Some(b)) = (has_branch, base.as_deref()) {
        args.push(b);
    }
    git(&p.root, &args).map_err(|e| CliError::Other(format!("git worktree add: {e}")))?;
    ris::project::exclude_locally(&p.root, &[ris::project::LOCAL_IMPORT_FILE])?;
    let import = path.join(ris::project::LOCAL_IMPORT_FILE);
    if !import.exists() {
        std::fs::write(&import, format!("@{}\n", p.rules_path().display()))?;
    }
    Ok(path)
}

/// `ris up`: make the harness match the configuration, doing only the
/// difference. Each named seat (or every seat) gets the worktree it
/// needs, brought up to the phase branch when idle and behind it, is
/// resumed if paused, and is started in the background if its
/// agent is not running. Seats in the log but not in the file are
/// reported and left alone.
/// `ris install`: the person's, outside any sandbox (D-48).
fn install(p: &Project, actor: &Actor) -> Result<Output, CliError> {
    if *actor != Actor::Person {
        return Err(CliError::Other(
            "refused: installing ris is the person's; a seat's ris runs in its sandbox and never writes the person's PATH\n  \
             next: merge the item; the person's next command installs the build"
                .into(),
        ));
    }
    let Some(cfg) = &p.config.install else {
        return Err(CliError::Other(
            "nothing to install: config.toml has no [install]\n  \
             next: add [install] with build, artifact and to (the template shows an example)"
                .into(),
        ));
    };
    let l = p.load()?;
    let Some((item, commit)) = ris::install::last_merged(&l.log) else {
        return Err(CliError::Other(
            "nothing to install: no item has been merged yet\n  next: `ris merge` an item first"
                .into(),
        ));
    };
    let commit = ris::install::install(&p.root, cfg, &commit)?;
    Ok(Output::of(
        format!(
            "installed ris from {} ({item}, the last merge) to {}",
            ris::install::short(&commit),
            cfg.to
        ),
        &schema::Installed {
            item,
            commit: commit.clone(),
            to: cfg.to.clone(),
        },
    ))
}

/// What the writing seats would lack to run the project's checks, read
/// from the configuration (`ris::preflight`); empty when the preflight
/// is off. PATH is the person's, which a seat inherits behind the
/// directory of `ris` itself.
fn preflight(p: &Project) -> Vec<ris::preflight::Missing> {
    if !p.config.runner.preflight {
        return Vec::new();
    }
    let path = std::env::var("PATH").ok();
    ris::preflight::check(&p.checks, &p.config.runner, path.as_deref())
}

fn up(p: &mut Project, actor: &Actor, named: &[String]) -> Result<Output, CliError> {
    if *actor != Actor::Person {
        return Err(CliError::Refused(
            ris_core::Refusal::SeatControlIsForThePerson,
        ));
    }
    for n in named {
        if !p.config.agents.contains_key(n) {
            return Err(CliError::UnknownAgent(n.clone()));
        }
    }
    let seats: Vec<String> = if named.is_empty() {
        p.config.agents.keys().cloned().collect()
    } else {
        named.to_vec()
    };
    // Before any seat starts: can the writing seats run the checks?
    // A seat that cannot would find out mid-item, unattended.
    if seats
        .iter()
        .any(|s| p.config.agents[s].start && p.config.agents[s].role.writes())
    {
        let missing = preflight(p);
        if !missing.is_empty() {
            return Err(CliError::Config(ris::preflight::render(&missing)));
        }
    }
    let mut changed: Vec<String> = close_checked(p, actor)?
        .into_iter()
        .filter(|c| c.contains("closed itself"))
        .collect();
    let l = p.load()?;
    // D-43: the phase branch is checked out nowhere. A main checkout
    // still on it, from the layout before, goes back to the main line
    // when that loses nothing.
    if let Some(phase) = ris::status::phase_branch(p, &l)
        && git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
            .as_deref()
            == Some(&phase)
    {
        let main = p.config.main_line.clone();
        let dirty = git(&p.root, &["status", "--porcelain", "--untracked-files=no"])
            .map_err(CliError::Other)?;
        if dirty.is_empty() && git(&p.root, &["switch", "--quiet", &main]).is_ok() {
            changed.push(format!(
                "main checkout: on {main}; {phase} is checked out nowhere and moves only by merges"
            ));
        } else {
            changed.push(format!(
                "main checkout: left on {phase}, which has uncommitted changes; \
                 seats' merges move it there until it is on {main}"
            ));
        }
    }
    // Every writing seat works in its own worktree: a builder always,
    // and the lead too with phase branches on, since the main checkout
    // then holds no phase work.
    let places = ris::status::seats(p, &l);
    for seat in &seats {
        let role = p.config.agents[seat].role;
        let place = places
            .iter()
            .find(|s| &s.seat == seat)
            .filter(|s| s.branch.as_deref() == Some(seat.as_str()));
        let writes = role == ris::config::Role::Builder
            || (role == ris::config::Role::Lead && p.config.phase_branches);
        if writes && place.and_then(|s| s.worktree.as_ref()).is_none() {
            let path = make_worktree(p, seat)?;
            changed.push(format!("{seat}: worktree at {}", shown_beside(p, &path)));
        }
    }
    // An attended seat has its worktree and nothing more: the person
    // running it decides when it moves, pauses and runs.
    let (seats, attended): (Vec<String>, Vec<String>) =
        seats.into_iter().partition(|s| p.config.agents[s].start);
    let name = |s: &String| AgentName(s.clone());
    let paused: Vec<AgentName> = seats
        .iter()
        .map(name)
        .filter(|s| l.state.paused.contains(s))
        .collect();
    if !paused.is_empty() {
        p.run(&Command::Resume {
            seats: paused.clone(),
            actor: actor.clone(),
        })?;
        for s in &paused {
            changed.push(format!("{s}: resumed"));
        }
    }
    // A run the log holds whose process is gone is cleared, so the seat
    // is started again below rather than counted as running for good.
    let cleared = clear_gone_runs(p, actor, &l, &seats)?;
    // Read again whatever was cleared: a run may also have recorded its
    // own stop since the first read, and its seat is then started below.
    let l = p.load()?;
    changed.extend(cleared);
    // After the cleared runs, so a seat whose process is gone does not
    // count as running when its worktree is brought up.
    changed.extend(bring_up_to_phase(p, &l, &seats));
    let exe = std::env::current_exe()?;
    let mut starting = Vec::new();
    for seat in &seats {
        if l.state.running.contains_key(&name(seat)) {
            continue;
        }
        // In the background, in its own process group, so it outlives
        // this command and the terminal's interrupt does not reach it;
        // it records its own start and stop. What it prints goes to a
        // file, read below if it fails early.
        use std::os::unix::process::CommandExt;
        let out_file = p
            .root
            .join(ris::project::DIR)
            .join("runs")
            .join(format!("{seat}.start"));
        if let Some(runs) = out_file.parent() {
            std::fs::create_dir_all(runs)?;
        }
        let out = std::fs::File::create(&out_file)?;
        let child = std::process::Command::new(&exe)
            .args(["seat", "start", seat, "--quiet"])
            .current_dir(&p.root)
            .env("RIS_AGENT", ris::project::PERSON)
            .stdin(std::process::Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .process_group(0)
            .spawn()?;
        starting.push((seat.clone(), child, out_file));
    }
    changed.extend(settle_starts(starting));
    let retired: Vec<String> = names_in_log(&l.state)
        .into_iter()
        .filter(|n| !p.config.agents.contains_key(n))
        .collect();
    let mut text = if changed.is_empty() {
        "nothing to do: the harness matches .osiris/config.toml".to_owned()
    } else {
        changed.join("\n")
    };
    if !retired.is_empty() {
        text.push_str(&format!(
            "\nin the log but not in the file, left alone: {}",
            retired.join(", ")
        ));
    }
    if !attended.is_empty() {
        text.push_str(&format!(
            "\nattended (start = false), left alone: {}",
            attended.join(", ")
        ));
    }
    Ok(Output::of(
        text,
        &schema::Up {
            changed,
            not_in_file: retired,
            attended,
        },
    ))
}

/// Bring each named writing seat's worktree that is behind the phase
/// branch up to it, so a seat never starts on a stale base. A
/// fast-forward, and only when nothing can be lost: the seat's branch
/// holds no commits of its own, it holds no claimed item, and its
/// worktree has no uncommitted or untracked files. A seat behind that
/// fails one of those is left alone and the line says why. The one
/// exception is a branch with commits of its own behind a change to a
/// rules file, which is rebased (`rebase_over_rules`). One line per
/// seat that was behind.
fn bring_up_to_phase(p: &mut Project, l: &ris::project::Loaded, named: &[String]) -> Vec<String> {
    let Some(phase) = ris::status::phase_branch(p, l) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for s in ris::status::seats(p, l) {
        if !named.contains(&s.seat) || !s.role.writes() {
            continue;
        }
        // Only a seat on its own branch in its own worktree: a lead at
        // width one reports the main checkout, which is not its to move.
        let (Some(dir), true) = (&s.worktree, s.branch.as_deref() == Some(s.seat.as_str())) else {
            continue;
        };
        let behind = s.behind.unwrap_or(0);
        if behind == 0 {
            continue;
        }
        let seat = &s.seat;
        // A branch with commits of its own is the seat's to rebase, at
        // its merge. It cannot when the phase branch changed a rules
        // file since, which its sandbox will not let it write: then the
        // rebase is done here, with the person's access.
        if s.ahead.unwrap_or(1) > 0
            && let Some(rules) = rules_changed_since_fork(dir, &phase)
        {
            lines.push(rebase_over_rules(p, dir, &s, &phase, behind, &rules));
            continue;
        }
        let held = if s.ahead.unwrap_or(1) > 0 {
            Some("its branch holds commits of its own".to_owned())
        } else if let Some(item) = &s.claim {
            Some(format!("it holds {item}"))
        } else if s.uncommitted.unwrap_or(1) > 0 {
            Some("its worktree has uncommitted changes".to_owned())
        } else {
            None
        };
        let line = match held {
            Some(why) => format!("{seat}: {behind} behind {phase}, left alone: {why}"),
            None => match git(dir, &["merge", "--quiet", "--ff-only", &phase]) {
                Ok(_) => format!("{seat}: brought up to {phase} ({behind} commits)"),
                Err(e) => format!(
                    "{seat}: {behind} behind {phase}, left alone: {}",
                    e.lines().next().unwrap_or("the fast-forward failed")
                ),
            },
        };
        lines.push(line);
    }
    lines
}

/// The rules files the phase branch has changed since the branch checked
/// out in `dir` left it, as a list for a message; `None` when it has
/// changed none.
fn rules_changed_since_fork(dir: &std::path::Path, phase: &str) -> Option<String> {
    let fork = git(dir, &["merge-base", "HEAD", phase]).ok()?;
    let mut args = vec!["diff", "--name-only", &fork, phase, "--"];
    let specs: Vec<String> = ris::runner::RULES_FILES
        .iter()
        .map(|f| format!(":(top){f}"))
        .collect();
    args.extend(specs.iter().map(String::as_str));
    let names = git(dir, &args).ok()?;
    (!names.is_empty()).then(|| names.lines().collect::<Vec<_>>().join(", "))
}

/// Rebase a seat's branch, which holds commits of its own, onto a phase
/// branch that has changed the rules files `rules` since: the one
/// rebase a seat cannot do for itself. Only when nothing can be lost
/// and nothing is in the way: the worktree has no uncommitted or
/// untracked files and the seat's agent is not running in it. Commits
/// that conflict with the phase branch outside the rules files are
/// handed back to the seat (`hand_back_conflict`), which resolves them
/// itself. The line says what was done, or why the seat was left alone.
fn rebase_over_rules(
    p: &mut Project,
    dir: &std::path::Path,
    s: &ris::status::Seat,
    phase: &str,
    behind: u32,
    rules: &str,
) -> String {
    let seat = &s.seat;
    let left = |why: String| {
        format!(
            "{seat}: {behind} behind {phase}, which changed {rules}, a rules file a seat cannot \
             rebase over; left alone: {why}"
        )
    };
    if s.uncommitted.unwrap_or(1) > 0 {
        return left(format!(
            "its worktree has uncommitted changes\n  next: commit or clear them, then `ris up {seat}`"
        ));
    }
    if s.running {
        return left(format!(
            "its agent is running\n  next: `ris up {seat}` once it has stopped"
        ));
    }
    if let RebaseOutlook::Conflict(files) = rebase_outlook(dir, phase) {
        // A conflict in a rules file is one the seat cannot resolve
        // either: its sandbox refuses the file. That one is the person's.
        if files
            .iter()
            .any(|f| ris::runner::RULES_FILES.contains(&f.as_str()))
        {
            return left(format!(
                "its commits conflict with {phase} in a rules file (in {})\n  \
                 next: `git rebase {phase}` in its worktree and resolve, then `ris up {seat}`",
                files.join(", ")
            ));
        }
        return hand_back_conflict(p, dir, s, phase, &files, &left);
    }
    if let Some(why) = git_identity_missing(dir, &format!("the rebase onto {phase}")) {
        return left(format!("{why}, then `ris up {seat}`"));
    }
    match git(dir, &["rebase", "--quiet", phase]) {
        Ok(_) => format!(
            "{seat}: rebased onto {phase} over its change to {rules}; its own commits are kept"
        ),
        Err(e) => {
            let _ = git(dir, &["rebase", "--abort"]);
            left(format!(
                "the rebase failed ({})\n  next: `git rebase {phase}` in its worktree, then `ris up {seat}`",
                e.lines().next().unwrap_or("no reason given")
            ))
        }
    }
}

/// The part of a conflicting rebase over a rules-file change that only
/// the person's `ris up` can do. The seat's commits are kept on a branch
/// named for the seat and its head, its worktree is reset to the phase
/// branch (which writes the rules files its sandbox refuses), and the
/// seat is sent a handoff naming the cherry-pick that brings its commits
/// back, so it resolves the conflict in its own worktree, where no rules
/// file is touched. The worktree is clean and its agent stopped, so the
/// kept branch holds everything the reset moves past.
fn hand_back_conflict(
    p: &mut Project,
    dir: &std::path::Path,
    s: &ris::status::Seat,
    phase: &str,
    files: &[String],
    left: &dyn Fn(String) -> String,
) -> String {
    let seat = &s.seat;
    let in_files = files.join(", ");
    let fallback = |why: String| {
        left(format!(
            "its commits conflict with {phase} (in {in_files}), and {why}\n  \
             next: `git rebase {phase}` in its worktree and resolve, then `ris up {seat}`"
        ))
    };
    let (Ok(head), Ok(fork)) = (
        git(dir, &["rev-parse", "HEAD"]),
        git(dir, &["merge-base", "HEAD", phase]),
    ) else {
        return fallback("its commits could not be read".to_owned());
    };
    let short = |c: &str| c.chars().take(12).collect::<String>();
    let kept = format!("{seat}-kept-{}", short(&head));
    // The name carries the head, so a branch already there under it from
    // an earlier run holds these same commits.
    let made = git(dir, &["branch", &kept, &head]).is_ok()
        || git(dir, &["rev-parse", &format!("refs/heads/{kept}")]).ok() == Some(head.clone());
    if !made {
        return fallback(format!("the branch {kept} could not be made to keep them"));
    }
    if let Err(e) = git(dir, &["reset", "--quiet", "--hard", phase]) {
        return fallback(format!(
            "the reset to {phase} failed ({}); its commits are on {kept}",
            e.lines().next().unwrap_or("no reason given")
        ));
    }
    // The item the commits are for: the seat's claim, or else the Item
    // trailer of its head commit.
    let item = s
        .claim
        .clone()
        .or_else(|| {
            git(
                dir,
                &[
                    "log",
                    "-1",
                    "--format=%(trailers:key=Item,valueonly)",
                    &head,
                ],
            )
            .ok()
        })
        .and_then(|i| i.trim().parse::<ItemId>().ok());
    let merge = match item {
        Some(i) => format!("`ris merge {i}`"),
        None => "`ris merge` the item".to_owned(),
    };
    let range = format!("{}..{kept}", short(&fork));
    let body = format!(
        "ris up moved your worktree to {phase}: your commits conflict with it (in {in_files}), \
         and it changed a rules file your sandbox cannot write, so you could not rebase. Your \
         commits are kept on the branch {kept}. Next, in your worktree: `git cherry-pick {range}`, \
         resolve the conflict in {in_files}, `git cherry-pick --continue`, keep one commit, \
         then {merge}."
    );
    let line = format!(
        "{seat}: moved to {phase}; its commits conflict with it (in {in_files}) and are kept on \
         {kept}; the seat cherry-picks them ({range}) and resolves the conflict itself"
    );
    let sent = p
        .load()
        .map(|l| item.filter(|i| l.state.items.contains_key(i)))
        .and_then(|about| {
            let body = p.text("body", &body)?;
            p.run(&Command::MessageSend {
                from: Actor::Person,
                to: AgentName(seat.clone()),
                kind: MessageKind::Handoff,
                about,
                body,
            })
        });
    match sent {
        Ok(_) => format!("{line}; told in its inbox"),
        Err(e) => format!(
            "{line}; the message to it failed ({})\n  next: tell {seat}: {body}",
            e.to_string().lines().next().unwrap_or("no reason given")
        ),
    }
}

/// Run the checks of open person actions, for the person only: their
/// commands are the person's, run with the person's access. A passing
/// check records the action done with its output as evidence; one that
/// fails changes nothing. What happened, one line per action.
fn close_checked(p: &mut Project, actor: &Actor) -> Result<Vec<String>, CliError> {
    if *actor != Actor::Person {
        return Ok(Vec::new());
    }
    let l = p.load()?;
    let open: Vec<(ris_core::ids::ActionId, String)> = l
        .state
        .actions
        .values()
        .filter(|a| a.state == ris_core::primitives::ActionState::Open)
        .filter_map(|a| a.check.map(|at| (a.id, l.text(at, "check").to_owned())))
        .collect();
    let mut lines = Vec::new();
    let limit = std::time::Duration::from_secs(p.config.action_check_seconds);
    for (id, check) in open {
        let (passed, output) = match checks::run_limited(&p.root, &check, limit)? {
            checks::Limited::Finished(passed, output) => (passed, output),
            checks::Limited::TimedOut { kill_fault } => {
                let mut line = format!(
                    "{id}: its check ran past {}s, so it does not pass yet",
                    limit.as_secs()
                );
                if let Some(fault) = kill_fault {
                    line.push_str(&format!(
                        "\n  what it started may still be running: stopping it failed ({fault})"
                    ));
                }
                lines.push(line);
                continue;
            }
        };
        if passed {
            let mut bytes = format!("$ {check}\n").into_bytes();
            bytes.extend(&output);
            let evidence = p.evidence.put(&bytes)?;
            p.run(&Command::ActionDone {
                action: id,
                by: Actor::Person,
                evidence: vec![evidence],
                note: Some(p.text("note", "closed by its check")?),
            })?;
            lines.push(format!("{id} closed itself: its check passed"));
        } else {
            lines.push(format!("{id}: its check does not pass yet"));
        }
    }
    Ok(lines)
}

/// `ris down`: pause each seat, so it finishes what it holds and its
/// agent stops at its next claim. Nothing is killed and nothing lost.
fn down(p: &mut Project, actor: &Actor, named: &[String]) -> Result<Output, CliError> {
    if *actor != Actor::Person {
        return Err(CliError::Refused(
            ris_core::Refusal::SeatControlIsForThePerson,
        ));
    }
    for n in named {
        if !p.config.agents.contains_key(n) {
            return Err(CliError::UnknownAgent(n.clone()));
        }
    }
    let seats: Vec<AgentName> = if named.is_empty() {
        p.config.agents.keys().cloned().map(AgentName).collect()
    } else {
        named.iter().cloned().map(AgentName).collect()
    };
    let (seats, attended): (Vec<AgentName>, Vec<AgentName>) =
        seats.into_iter().partition(|s| p.config.agents[&s.0].start);
    let attended: Vec<String> = attended.into_iter().map(|s| s.0).collect();
    let note = if attended.is_empty() {
        String::new()
    } else {
        format!(
            "\nattended (start = false), left alone: {}",
            attended.join(", ")
        )
    };
    let l = p.load()?;
    let to_pause: Vec<AgentName> = seats
        .into_iter()
        .filter(|s| !l.state.paused.contains(s))
        .collect();
    if to_pause.is_empty() {
        return Ok(Output::of(
            format!("nothing to do: every seat named is already down{note}"),
            &schema::Down {
                changed: Vec::new(),
                attended,
            },
        ));
    }
    let (l, _) = p.run(&Command::Pause {
        seats: to_pause.clone(),
        actor: actor.clone(),
    })?;
    let changed: Vec<String> = to_pause
        .iter()
        .map(|s| {
            let holds = l.state.items.values().find(|i| {
                matches!(&i.state, ris_core::primitives::ItemState::Claimed { agent } if agent == s)
            });
            match (holds, l.state.running.contains_key(s)) {
                (Some(i), _) => format!("{s}: paused; finishes {} first", i.id),
                (None, true) => format!("{s}: paused; its agent stops at its next claim"),
                (None, false) => format!("{s}: paused"),
            }
        })
        .collect();
    Ok(Output::of(
        format!("{}{note}", changed.join("\n")),
        &schema::Down { changed, attended },
    ))
}

/// `ris logs`: the seat's latest run, with the agent tool's stream
/// rendered as text where it is understood and shown raw otherwise.
fn logs(p: &Project, seat: &str, follow: bool, json: bool) -> Result<Output, CliError> {
    if !p.config.agents.contains_key(seat) {
        return Err(CliError::UnknownAgent(seat.to_owned()));
    }
    let dir = p.root.join(ris::project::DIR).join("runs");
    let Some(path) = ris::runner::latest_run(&dir, seat) else {
        return Ok(Output::of(
            format!("{seat} has not run yet\n  next: `ris up {seat}`"),
            &schema::Logs::Seat {
                seat: seat.to_owned(),
                lines: Vec::new(),
            },
        ));
    };
    let mut at = 0;
    let mut all = Vec::new();
    loop {
        // Whether the run is still going is read before the file, so
        // the last read after it stops sees everything it wrote, and
        // only then is a last line without a newline taken as it is.
        let running = follow
            && p.load()?
                .state
                .running
                .contains_key(&AgentName(seat.to_owned()));
        let text = std::fs::read_to_string(&path)?;
        let (lines, next) = ris::runner::lines_from(&text, at, !running);
        at = next;
        for line in lines {
            let Some(shown) = ris::runner::render_line(line) else {
                continue;
            };
            if follow {
                // Followed output goes out as it comes: with --json, one
                // envelope per line, as `ris watch` streams events.
                if json {
                    emit(
                        &render::envelope(&schema::Logs::Line(schema::SeatLine {
                            seat: seat.to_owned(),
                            line: shown.clone(),
                        }))
                        .to_string(),
                    );
                } else {
                    emit(&shown);
                }
            }
            all.push(shown);
        }
        if !running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if follow {
        // Everything has been printed; nothing follows it.
        return Ok(Output::stream::<schema::Logs>(String::new(), &[]));
    }
    Ok(Output::of(
        all.join("\n"),
        &schema::Logs::Seat {
            seat: seat.to_owned(),
            lines: all,
        },
    ))
}

/// `ris logs` with no seat: every configured seat's latest run, each
/// line prefixed with its seat as `docker compose logs` does. Followed,
/// each seat is read from where it was left, a seat's newer run is
/// picked up when it starts, and it ends once no seat is running and
/// what they wrote has been read.
fn logs_all(p: &Project, follow: bool, json: bool) -> Result<Output, CliError> {
    let dir = p.root.join(ris::project::DIR).join("runs");
    let seats: Vec<String> = p.config.agents.keys().cloned().collect();
    let width = seats.iter().map(|s| s.len()).max().unwrap_or(0);
    // Per seat, the run file being read and how far.
    let mut at: std::collections::BTreeMap<String, (std::path::PathBuf, usize)> =
        std::collections::BTreeMap::new();
    let mut all = Vec::new();
    let mut text = Vec::new();
    loop {
        // As for one seat: who is running is read before the files.
        let running = if follow {
            p.load()?.state.running
        } else {
            Default::default()
        };
        for seat in &seats {
            let Some(path) = ris::runner::latest_run(&dir, seat) else {
                continue;
            };
            let from = match at.get(seat) {
                Some((read, n)) if *read == path => *n,
                _ => 0,
            };
            let Ok(file) = std::fs::read_to_string(&path) else {
                continue;
            };
            let live = running.contains_key(&AgentName(seat.clone()));
            let (lines, next) = ris::runner::lines_from(&file, from, !live);
            at.insert(seat.clone(), (path, next));
            for line in lines {
                let Some(shown) = ris::runner::render_line(line) else {
                    continue;
                };
                for part in shown.lines() {
                    let prefixed = format!("{seat:<width$} | {part}");
                    let line = schema::SeatLine {
                        seat: seat.clone(),
                        line: part.to_owned(),
                    };
                    if follow {
                        if json {
                            emit(&render::envelope(&schema::Logs::Line(line.clone())).to_string());
                        } else {
                            emit(&prefixed);
                        }
                    }
                    all.push(line);
                    text.push(prefixed);
                }
            }
        }
        if !seats
            .iter()
            .any(|s| running.contains_key(&AgentName(s.clone())))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if follow {
        return Ok(Output::stream::<schema::Logs>(String::new(), &[]));
    }
    if text.is_empty() {
        return Ok(Output::of(
            "no seat has run yet\n  next: `ris up`".to_owned(),
            &schema::Logs::All { lines: Vec::new() },
        ));
    }
    Ok(Output::of(
        text.join("\n"),
        &schema::Logs::All { lines: all },
    ))
}

/// A path beside the repository, shown relative to its parent.
fn shown_beside(p: &Project, path: &std::path::Path) -> String {
    ris::config::relative(&p.root, path).display().to_string()
}

/// The goal of the item every phase gets for its docs (`docs_item`).
const DOCS_GOAL: &str = "Docs describe the system as built: the README, the quickstart, the \
    JSON contract, the agents' rules and the design's status match this phase's work";

/// `ris seat start`: run a seat's agent through the runner and record
/// its start and stop around it.
fn start_seat(p: &mut Project, actor: &Actor, seat: &str, quiet: bool) -> Result<Output, CliError> {
    let cfg = p
        .config
        .agents
        .get(seat)
        .cloned()
        .ok_or_else(|| CliError::UnknownAgent(seat.to_owned()))?;
    if let Some(r) = cfg.runner.as_deref().filter(|r| *r != "claude") {
        return Err(CliError::Config(format!(
            "seat {seat} names runner `{r}`; the only runner adapter is `claude`"
        )));
    }
    let l = p.load()?;
    let dir = ris::status::seats(p, &l)
        .into_iter()
        .find(|s| s.seat == seat)
        .and_then(|s| s.worktree)
        .ok_or_else(|| {
            CliError::Other(format!(
                "{seat} has nowhere to work yet\n  next: `ris worktree {seat}`"
            ))
        })?;
    let abs = |dir: &std::path::Path, what: &str| {
        git(dir, &["rev-parse", "--path-format=absolute", what])
            .ok()
            .map(PathBuf::from)
    };
    let git_dirs = abs(&dir, "--git-common-dir").map(|common| {
        // The seat's own entry under worktrees/, when its directory is
        // a linked worktree rather than the main checkout.
        let own = abs(&dir, "--git-dir").filter(|g| *g != common);
        ris::runner::GitDirs { common, own }
    });
    let mut write: Vec<String> = ris::runner::writable(&p.root, cfg.role, git_dirs.as_ref())
        .iter()
        .map(|w| w.display().to_string())
        .collect();
    write.extend(p.config.runner.write.iter().cloned());
    let extra = p
        .config
        .runner
        .allow
        .get(&cfg.role.to_string())
        .cloned()
        .unwrap_or_default();
    let allow = ris::runner::allowlist(cfg.role, &p.checks, &extra);
    let mut protected = ris::runner::protected(&p.root, &p.rules_path(), &dir);
    if let Some(g) = &git_dirs {
        protected.extend(ris::runner::git_protected(g, &dir));
    }
    let deny = ris::runner::deny_edits(&protected);
    let protected: Vec<String> = protected.iter().map(|w| w.display().to_string()).collect();
    let settings = ris::runner::sandbox_settings(&write, &protected, &p.config.runner.network);
    let prompt = ris::runner::prompt(seat, cfg.role);
    let args = ris::runner::claude_args(&prompt, &allow, &deny, &settings, cfg.model.as_deref());

    let name = AgentName(seat.to_owned());
    let (l, _) = p.run(&Command::RunStart {
        seat: name.clone(),
        actor: actor.clone(),
        runner: p.text("runner", "claude")?,
        model: cfg
            .model
            .as_deref()
            .map(|m| p.text("model", m))
            .transpose()?,
    })?;
    let at = l.state.running[&name];
    let log = p
        .root
        .join(ris::project::DIR)
        .join("runs")
        .join(format!("{seat}-{at}.jsonl"));
    let shown_log = log
        .strip_prefix(&p.root)
        .unwrap_or(&log)
        .display()
        .to_string();
    let program = p.config.runner.program.clone();
    // The run holds a lock for as long as this process lives, so `ris
    // up` can tell a run whose process is gone (killed, or the machine
    // restarted) from a live one: the operating system releases the
    // lock however the process ends. The file names the process.
    let lock_path = run_lock_file(p, seat);
    if let Some(runs) = lock_path.parent() {
        std::fs::create_dir_all(runs)?;
    }
    let held = std::fs::File::create(&lock_path)?;
    held.try_lock().map_err(|e| {
        CliError::Other(format!("{seat}'s run lock is held by another process: {e}"))
    })?;
    std::fs::write(&lock_path, format!("{}\n", std::process::id()))?;
    let result = ris::runner::run(&program, &args, &dir, seat, &p.root, &log, quiet);
    let status = result.as_ref().ok().copied().flatten();
    p.run(&Command::RunStop {
        seat: name,
        actor: actor.clone(),
        status,
    })?;
    drop(held);
    let status = result.map_err(|e| {
        CliError::Other(format!(
            "could not run `{program}` for {seat}: {e}\n  next: install it, or set [runner] program in .osiris/config.toml"
        ))
    })?;
    Ok(Output::of(
        format!(
            "{seat} stopped ({}); its output is in {shown_log}",
            status.map_or("ended by a signal".to_owned(), |c| format!("exit {c}"))
        ),
        &schema::SeatStopped {
            seat: seat.to_owned(),
            status,
            log: shown_log.clone(),
        },
    ))
}

/// How long `ris up` watches the seats it started before saying they
/// started: a start that fails (no agent tool, nowhere to work) fails
/// well within it.
const START_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Watch the `ris seat start` processes just spawned for `START_GRACE`,
/// or until all have exited, and say for each whether it started,
/// failed (with the first line of what it printed), or already ended.
fn settle_starts(mut starting: Vec<(String, std::process::Child, PathBuf)>) -> Vec<String> {
    let deadline = std::time::Instant::now() + START_GRACE;
    let mut exited: Vec<Option<std::process::ExitStatus>> = vec![None; starting.len()];
    while std::time::Instant::now() < deadline && exited.iter().any(Option::is_none) {
        for (i, (_, child, _)) in starting.iter_mut().enumerate() {
            if exited[i].is_none() {
                exited[i] = child.try_wait().ok().flatten();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    starting
        .iter()
        .zip(exited)
        .map(|((seat, _, out_file), status)| match status {
            None => format!("{seat}: started; `ris logs {seat}` follows it"),
            Some(s) => {
                let printed = std::fs::read_to_string(out_file).unwrap_or_default();
                let first = printed.lines().next().unwrap_or("").trim().to_owned();
                if s.success() {
                    format!("{seat}: started and already stopped: {first}")
                } else {
                    format!("{seat}: failed to start: {first}")
                }
            }
        })
        .collect()
}

/// The lock a seat's live run holds, which also names its process.
fn run_lock_file(p: &Project, seat: &str) -> PathBuf {
    p.root
        .join(ris::project::DIR)
        .join("runs")
        .join(format!("{seat}.lock"))
}

/// Whether the process holding a seat's run is alive: whether its run
/// lock is still held. In doubt (no lock file, as for a run started
/// before run locks were kept, or a lock that cannot be tried), alive:
/// a live run must never be cleared.
fn run_alive(p: &Project, seat: &str) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .open(run_lock_file(p, seat))
    else {
        return true;
    };
    !matches!(file.try_lock(), Ok(()))
}

/// Clear each named seat's run whose process is gone: the log says it
/// is running, but no `ris seat start` holds it, so it would stay
/// running for good and never be started again. One line per seat.
fn clear_gone_runs(
    p: &mut Project,
    actor: &Actor,
    l: &ris::project::Loaded,
    named: &[String],
) -> Result<Vec<String>, CliError> {
    let mut lines = Vec::new();
    for seat in named {
        let name = AgentName(seat.clone());
        if !l.state.running.contains_key(&name) || run_alive(p, seat) {
            continue;
        }
        match p.run(&Command::RunStop {
            seat: name,
            actor: actor.clone(),
            status: None,
        }) {
            Ok(_) => lines.push(format!(
                "{seat}: its last run ended without being recorded (its process is gone); cleared"
            )),
            // `l` was read before this: the run recorded its own stop
            // since, on its way out, and there is nothing left to clear.
            Err(CliError::Refused(ris_core::Refusal::NotRunning(_))) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(lines)
}

/// Width one: the item must be exactly one commit on top of where the
/// phase branch stood after its last recorded merge (or where the phase
/// branch left the main line, before the phase's first merge), as a
/// merge from a worktree requires. Skipped only when there is no base
/// to count from: work committed straight onto the main line with no
/// merge yet recorded in the phase, or no main line to fork from.
fn require_one_commit(
    p: &Project,
    l: &ris::project::Loaded,
    item: ItemId,
    commit: &str,
) -> Result<(), CliError> {
    let Some(phase) = l.state.open_phase() else {
        return Ok(());
    };
    let count = |base: &str| -> Option<u32> {
        git(
            &p.work,
            &["rev-list", "--count", &format!("{base}..{commit}")],
        )
        .ok()?
        .parse()
        .ok()
    };
    let is_ancestor =
        |base: &str| git(&p.work, &["merge-base", "--is-ancestor", base, commit]).is_ok();
    // The nearest merge recorded since the phase opened that the commit
    // builds on, whichever phase the merged item was added in: items
    // carry over from phase to phase.
    let merged = l
        .log
        .iter()
        .filter(|e| e.seq > phase.at)
        .filter_map(|e| match &e.structure {
            ris_core::event::Structure::ItemMerged { commit: c, .. } => Some(c.0.clone()),
            _ => None,
        })
        .filter(|c| c != commit && is_ancestor(c))
        .filter_map(|c| count(&c).map(|n| (n, c)))
        .min();
    let (n, base) = match merged {
        Some((n, c)) => (n, format!("the last merge ({})", &c[..c.len().min(8)])),
        None => {
            // On the main line itself there is no fork point to count from.
            let branch = git(&p.work, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            if branch == p.config.main_line {
                return Ok(());
            }
            let Ok(fork) = git(&p.work, &["merge-base", &p.config.main_line, commit]) else {
                return Ok(());
            };
            match count(&fork) {
                Some(n) => (n, format!("where the phase left {}", p.config.main_line)),
                None => return Ok(()),
            }
        }
    };
    match n {
        1 => Ok(()),
        0 => Err(CliError::Other(format!(
            "nothing to merge: no commit for {item} beyond {base}\n  next: commit the item first"
        ))),
        n => Err(CliError::Other(format!(
            "refused: {n} commits since {base}; {item} merges as one commit\n  \
             next: squash them (`git reset --soft` to that commit, then commit once with the trailers)"
        ))),
    }
}

/// What rebasing a branch onto another would meet.
enum RebaseOutlook {
    /// No conflict. `rules` are the rules files (`RULES_FILES`, from the
    /// top of the tree) the rebase would rewrite in the worktree.
    Clean { rules: Vec<String> },
    /// The files that conflict.
    Conflict(Vec<String>),
    /// Git could not say (one older than 2.38 has no `merge-tree
    /// --write-tree`): the rebase itself is the test.
    Unknown,
}

/// What rebasing `HEAD` in `dir` onto `onto` would meet, worked out in
/// the object store alone, so neither the worktree nor the index is
/// touched. The two are merged as trees, which for a branch of one
/// commit is the merge its rebase performs.
fn rebase_outlook(dir: &std::path::Path, onto: &str) -> RebaseOutlook {
    let Ok(out) = std::process::Command::new("git")
        .args(["merge-tree", "--write-tree", "--name-only", "--no-messages"])
        .args([onto, "HEAD"])
        .current_dir(dir)
        .output()
    else {
        return RebaseOutlook::Unknown;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let Some(tree) = lines.next() else {
        return RebaseOutlook::Unknown;
    };
    match out.status.code() {
        Some(0) => {
            let mut args = vec!["diff", "--name-only", "HEAD", tree, "--"];
            let specs: Vec<String> = ris::runner::RULES_FILES
                .iter()
                .map(|f| format!(":(top){f}"))
                .collect();
            args.extend(specs.iter().map(String::as_str));
            match git(dir, &args) {
                Ok(names) => RebaseOutlook::Clean {
                    rules: names.lines().map(str::to_owned).collect(),
                },
                Err(_) => RebaseOutlook::Unknown,
            }
        }
        Some(1) => RebaseOutlook::Conflict(
            lines
                .take_while(|l| !l.is_empty())
                .map(str::to_owned)
                .collect(),
        ),
        _ => RebaseOutlook::Unknown,
    }
}

/// Whether this process may write `path`, found without changing it: a
/// file that exists is opened for appending and nothing is written; one
/// that does not is created and removed. A seat's sandbox refuses both
/// for a rules file.
fn can_write(path: &std::path::Path) -> bool {
    use std::fs::OpenOptions;
    if path.exists() {
        return OpenOptions::new().append(true).open(path).is_ok();
    }
    // Without its directory there is nothing to try; the rebase decides.
    if !path.parent().is_some_and(|d| d.is_dir()) {
        return true;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => {
            let _ = std::fs::remove_file(path);
            true
        }
        Err(_) => false,
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
    let l = p.load()?;
    let phase = match ris::status::phase_branch(p, &l) {
        Some(b) => b,
        None => {
            let here =
                git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(CliError::Other)?;
            if here == "HEAD" {
                return Err(CliError::Other(
                    "the main worktree is detached; check out the branch merges land on there"
                        .into(),
                ));
            }
            here
        }
    };
    let old =
        git(&p.root, &["rev-parse", &format!("refs/heads/{phase}")]).map_err(CliError::Other)?;
    // What the rebase would meet is found before it starts, so a merge
    // that cannot go through leaves the worktree as it was. A branch
    // already on top of the phase branch has nothing to rebase.
    if git(&p.work, &["merge-base", "--is-ancestor", &phase, "HEAD"]).is_err() {
        require_git_identity(&p.work, &format!("the rebase onto {phase}"))?;
        match rebase_outlook(&p.work, &phase) {
            RebaseOutlook::Conflict(files) => {
                return Err(CliError::Other(format!(
                    "refused: {item} conflicts with work merged to {phase} before it (in {})\n  \
                     next: `git rebase {phase}` in your worktree, resolve, keep one commit, and run `ris merge {item}` again",
                    files.join(", ")
                )));
            }
            RebaseOutlook::Clean { rules } => {
                let top = git(&p.work, &["rev-parse", "--show-toplevel"])
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| p.work.clone());
                let locked: Vec<String> = rules
                    .into_iter()
                    .filter(|f| !can_write(&top.join(f)))
                    .collect();
                if !locked.is_empty() {
                    return Err(CliError::Other(format!(
                        "refused: {phase} changed {} since your branch left it, and this seat \
                         cannot write a rules file, so it cannot rebase onto {phase}\n  \
                         next: stop and say so; the person's `ris up {author}` brings your branch \
                         up to {phase}, and then `ris merge {item}` goes through",
                        locked.join(", ")
                    )));
                }
            }
            RebaseOutlook::Unknown => {}
        }
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
    match ris::status::checked_out(&p.root, &phase) {
        // Checked out somewhere (the layout before D-43): move it there,
        // files and all.
        Some(dir) => {
            git(&dir, &["merge", "--quiet", "--ff-only", &commit]).map_err(|e| {
                CliError::Other(format!(
                    "could not move {phase} to {commit}: {e}\n  \
                     next: {phase} is checked out in {}; switch that checkout to {}, \
                     or clear the changes in the way",
                    shown_beside(p, &dir),
                    p.config.main_line
                ))
            })?;
        }
        // Checked out nowhere: a ref update, which writes only the git
        // directory, and refuses if the branch moved since `old`.
        None => {
            git(
                &p.root,
                &[
                    "update-ref",
                    "-m",
                    &format!("ris merge {item}"),
                    &format!("refs/heads/{phase}"),
                    &commit,
                    &old,
                ],
            )
            .map_err(|e| {
                CliError::Other(format!(
                    "could not move {phase} to {commit}: {e}\n  next: run `ris merge {item}` again"
                ))
            })?;
        }
    }
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
    let here = git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(CliError::Other)?;
    // The phase's branch, checked out nowhere since D-43; without one,
    // whatever the main checkout is on (phase branches off, or a layout
    // from before).
    let named = phase_branch_name(phase);
    let branch = if p.config.phase_branches
        && git(
            &p.root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{named}"),
            ],
        )
        .is_ok()
    {
        named
    } else {
        here.clone()
    };
    if branch == main {
        return Ok(format!(
            "closing merge: none needed; the phase's work is already on {main}"
        ));
    }
    if here == "HEAD" {
        return Err(CliError::Other(format!(
            "the main worktree is detached; check out {main} there"
        )));
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
    require_git_identity(
        &p.root,
        &format!("the closing merge of {branch} into {main}"),
    )?;
    let switched = here != main;
    if switched {
        git(&p.root, &["switch", "--quiet", &main])
            .map_err(|e| CliError::Other(format!("cannot switch to the main line {main}: {e}")))?;
    }
    let back = |p: &Project| {
        if switched {
            let _ = git(&p.root, &["switch", "--quiet", &here]);
        }
    };
    let undo = |p: &Project| {
        let _ = git(&p.root, &["merge", "--abort"]);
        let _ = git(&p.root, &["reset", "--quiet", "--hard", "ORIG_HEAD"]);
        back(p);
    };
    let message = format!("chore: close {phase}: {goal}");
    if let Err(e) = git(
        &p.root,
        &["merge", "--quiet", "--no-ff", "-m", &message, &branch],
    ) {
        let _ = git(&p.root, &["merge", "--abort"]);
        back(p);
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

/// With phase branches on, create `phase/P-n` from the main line. It is
/// checked out nowhere: seats work in their own worktrees and merges move
/// it by a ref update (D-43), and the main checkout stays where it is. Returns the branch, or `None` when
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
    git(&p.root, &["branch", &name, main])
        .map_err(|e| CliError::Other(format!("could not create {name} from {main}: {e}")))?;
    Ok(Some(name))
}

#[cfg(test)]
#[path = "tests/main.rs"]
mod tests;
