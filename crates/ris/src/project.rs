//! A project: the directory with `.osiris/` in it, and everything the
//! shell needs to run a command against it.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ris_core::command::{Command, apply, seal};
use ris_core::event::{Event, Salt};
use ris_core::ids::{Actor, AgentName};
use ris_core::refusal::Refusal;
use ris_core::state::State;
use ris_core::store::{Store, StoreError};
use ris_core::text::{Policy, Text};
use ris_store_sqlite::SqliteStore;

use crate::config::{ChecksConfig, Config};
use crate::evidence::EvidenceStore;

/// The directory name.
pub const DIR: &str = ".osiris";

/// The rules file `ris init` writes at the project root.
pub const RULES_FILE: &str = "OSIRIS.md";

/// The value of `RIS_AGENT` that means the person. Reserved: no seat
/// may take it.
pub const PERSON: &str = "person";

/// The agents' rules, generic to any project.
pub const RULES: &str = include_str!("rules.md");

/// The agent tool's local, uncommitted rules file, which imports the
/// rules in a local setup.
pub const LOCAL_IMPORT_FILE: &str = "CLAUDE.local.md";

/// Add `lines` to git's local exclude file (shared by every worktree,
/// never committed) unless already there.
pub fn exclude_locally(root: &Path, lines: &[&str]) -> Result<(), CliError> {
    let path = git_path(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ],
    )
    .ok_or_else(|| CliError::Other("cannot find git's info/exclude".into()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    for line in lines {
        if !existing.lines().any(|l| l.trim() == *line) {
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(line);
            existing.push('\n');
        }
    }
    fs::write(&path, existing)?;
    Ok(())
}

/// Why the shell could not do what was asked.
#[derive(Debug)]
pub enum CliError {
    /// The core refused the command. Exit status 2.
    Refused(Refusal),
    Store(StoreError),
    /// No `.osiris/` here or above.
    NotAProject(PathBuf),
    /// `ris init` where one already exists.
    AlreadyAProject(PathBuf),
    Config(String),
    /// `RIS_AGENT` names a seat `config.toml` does not.
    UnknownAgent(String),
    /// `RIS_AGENT` is unset: nobody said who is acting. Never taken to
    /// mean the person, so an agent that loses its seat is refused
    /// rather than given the person's powers. Holds the seat names.
    NoActor(Vec<String>),
    /// The seat's role does not do this: an auditor reads and never
    /// writes to the tree; an advisor adds items, sends messages and
    /// raises decisions for the person, and nothing else. Exit status
    /// 2, like any refusal.
    RoleForbids {
        seat: String,
        role: crate::config::Role,
        what: &'static str,
    },
    /// Two writers kept colliding; the caller may retry.
    Contended,
    /// The exit scan found something in a commit or in files. Exit
    /// status 2, like any confidentiality refusal. Hits name a place and
    /// a category, never the text.
    Leak {
        /// What was scanned: a commit's short hash, or "working tree".
        what: String,
        hits: Vec<crate::exits::Hit>,
    },
    /// `ris check` found a failing check. Exit status 3. Nothing was
    /// recorded.
    CheckFailed {
        lines: Vec<String>,
    },
    /// One or more checks failed. Exit status 3. The runs are in the
    /// log and their output is evidence.
    ChecksFailed {
        item: ris_core::ids::ItemId,
        /// What the item did not become: "done" or "merged".
        stage: &'static str,
        failed: Vec<(String, ris_core::ids::EvidenceId)>,
        /// The per-check lines already composed for printing.
        lines: Vec<String>,
    },
    Io(std::io::Error),
    /// Anything else, in one line.
    Other(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "{r}\n  next: {}", r.next_step()),
            Self::Store(e) => write!(f, "{e}"),
            Self::NotAProject(p) => write!(
                f,
                "no {DIR}/ in {} or above\n  next: `ris init` in the project root",
                p.display()
            ),
            Self::AlreadyAProject(p) => write!(f, "{} already exists", p.display()),
            Self::Config(e) => write!(f, "{e}"),
            Self::UnknownAgent(a) => write!(
                f,
                "RIS_AGENT={a} is not a seat in {DIR}/config.toml\n  next: add the seat with `ris seat add`, or set RIS_AGENT={PERSON} to act as the person"
            ),
            Self::NoActor(seats) => write!(
                f,
                "refused: RIS_AGENT is not set, so ris does not know who is acting\n  next: set RIS_AGENT={PERSON} if you are the person (once, in your shell profile), or RIS_AGENT=<seat> for an agent{}",
                if seats.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", seats.join(", "))
                }
            ),
            Self::RoleForbids { seat, role, what } => {
                use crate::config::Role;
                // Matched exhaustively, so a new role must say its own
                // next step.
                let (a, next) = match role {
                    Role::Advisor => (
                        "an",
                        "`ris item add`, `ris send` or `ris decision raise --substantive`; \
                         the person or a writing seat does this",
                    ),
                    Role::Auditor => (
                        "an",
                        "`ris audit` for the review queue; a builder or lead seat does this",
                    ),
                    Role::Lead | Role::Builder => (
                        "a",
                        "check the seat's role in .osiris/config.toml; `ris status` shows it",
                    ),
                };
                write!(
                    f,
                    "refused: {seat} is {a} {role}, and {a} {role} does not {what}\n  next: {next}"
                )
            }
            Self::Contended => write!(f, "the log kept moving under this command; try again"),
            Self::Leak { what, hits } => {
                writeln!(
                    f,
                    "refused: {what} contains what the ground rule keeps out:"
                )?;
                for h in hits {
                    writeln!(f, "  {h}")?;
                }
                write!(
                    f,
                    "  next: remove it (a test fixture assembles its canary at runtime), \
                     rotate anything that was a real secret, and run again"
                )
            }
            Self::CheckFailed { lines } => write!(f, "{}", lines.join("\n")),
            Self::ChecksFailed {
                item,
                stage,
                failed,
                lines,
            } => {
                for l in lines {
                    writeln!(f, "{l}")?;
                }
                write!(
                    f,
                    "{item} not {stage}: {} check(s) failed\n  next: read the evidence ({}), fix, and run again",
                    failed.len(),
                    failed
                        .iter()
                        .map(|(_, e)| e.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Self::Io(e) => write!(f, "{e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<Refusal> for CliError {
    fn from(r: Refusal) -> Self {
        Self::Refused(r)
    }
}

impl From<StoreError> for CliError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// The log and its fold, with text available by event.
pub struct Loaded {
    pub state: State,
    pub log: Vec<Event>,
    by_seq: BTreeMap<u64, usize>,
}

impl Loaded {
    fn new(log: Vec<Event>, state: State) -> Self {
        let by_seq = log.iter().enumerate().map(|(i, e)| (e.seq, i)).collect();
        Self { state, log, by_seq }
    }

    /// A text field of the event at `seq`, if the event carries it and
    /// it has not been removed: for optional fields.
    pub fn log_text(&self, seq: u64, field: &str) -> Option<&str> {
        self.by_seq
            .get(&seq)
            .and_then(|i| self.log[*i].text_field(field))
            .map(Text::as_str)
    }

    /// A text field of the event at `seq`, or a marker if the text has
    /// been removed.
    pub fn text(&self, seq: u64, field: &str) -> &str {
        self.by_seq
            .get(&seq)
            .and_then(|i| self.log[*i].text_field(field))
            .map(Text::as_str)
            .unwrap_or("[text removed]")
    }
}

/// An open project.
pub struct Project {
    /// Where `.osiris/` lives: the main worktree of the repository.
    pub root: PathBuf,
    /// The caller's own worktree: where checks run and where its commits
    /// are read. The same as `root` at width one.
    pub work: PathBuf,
    pub config: Config,
    pub checks: ChecksConfig,
    pub policy: Policy,
    pub evidence: EvidenceStore,
    store: SqliteStore,
}

impl Project {
    /// Where the agents' rules live: inside `.osiris/` in a local setup,
    /// at the root otherwise.
    pub fn rules_path(&self) -> PathBuf {
        let local = self.root.join(DIR).join(RULES_FILE);
        if local.exists() {
            local
        } else {
            self.root.join(RULES_FILE)
        }
    }

    /// Walk up from `start` to the directory holding `.osiris/`.
    /// Find the store from `start`: `RIS_ROOT` if set, else the nearest
    /// directory above with `.osiris/`, else the main worktree of the
    /// git repository `start` is in (so every worktree of a repository
    /// shares one store). Checks run in `start`'s own worktree.
    pub fn find(start: &Path) -> Result<Self, CliError> {
        let root = Self::locate(start).ok_or_else(|| CliError::NotAProject(start.to_path_buf()))?;
        let mut p = Self::open(&root)?;
        if let Some(work) = git_path(start, &["rev-parse", "--show-toplevel"]) {
            p.work = work;
        }
        Ok(p)
    }

    fn locate(start: &Path) -> Option<PathBuf> {
        if let Some(root) = env::var_os("RIS_ROOT") {
            let root = PathBuf::from(root);
            return root.join(DIR).is_dir().then_some(root);
        }
        let mut dir = Some(start);
        while let Some(d) = dir {
            if d.join(DIR).is_dir() {
                return Some(d.to_path_buf());
            }
            dir = d.parent();
        }
        let common = git_path(
            start,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let main = common.parent()?.to_path_buf();
        main.join(DIR).is_dir().then_some(main)
    }

    /// Create `.osiris/` in `root` from the templates.
    /// Create `.osiris/`. With `local`, nothing tracked changes: the
    /// rules go inside `.osiris/`, and the ignore lines go in git's
    /// local exclude file, which is never committed.
    pub fn init(root: &Path, local: bool) -> Result<Self, CliError> {
        let dir = root.join(DIR);
        if dir.exists() {
            return Err(CliError::AlreadyAProject(dir));
        }
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("config.toml"), Config::TEMPLATE)?;
        fs::write(dir.join("checks.toml"), ChecksConfig::TEMPLATE)?;
        if local {
            fs::write(dir.join(RULES_FILE), RULES)?;
            if git_path(root, &["rev-parse", "--git-dir"]).is_some() {
                exclude_locally(root, &[&format!("{DIR}/"), LOCAL_IMPORT_FILE])?;
                // The agent tool's local rules file imports the rules.
                // An existing one is left alone.
                let import = root.join(LOCAL_IMPORT_FILE);
                if !import.exists() {
                    fs::write(&import, format!("@{DIR}/{RULES_FILE}\n"))?;
                }
            }
            return Self::open(root);
        }
        // The agents' rules, generic to any project. Never overwritten.
        let rules = root.join(RULES_FILE);
        if !rules.exists() {
            fs::write(&rules, RULES)?;
        }
        // The store stays out of git. Export, when wanted, is explicit.
        if root.join(".git").exists() {
            let gitignore = root.join(".gitignore");
            let existing = fs::read_to_string(&gitignore).unwrap_or_default();
            if !existing.lines().any(|l| l.trim() == format!("{DIR}/")) {
                let sep = if existing.is_empty() || existing.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                fs::write(&gitignore, format!("{existing}{sep}{DIR}/\n"))?;
            }
        }
        Self::open(root)
    }

    fn open(root: &Path) -> Result<Self, CliError> {
        let dir = root.join(DIR);
        let config = Config::parse(&fs::read_to_string(dir.join("config.toml"))?)
            .map_err(CliError::Config)?;
        config.check_agent_names().map_err(CliError::Config)?;
        let checks = match fs::read_to_string(dir.join("checks.toml")) {
            Ok(t) => ChecksConfig::parse(&t).map_err(CliError::Config)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ChecksConfig::default(),
            Err(e) => return Err(e.into()),
        };
        let policy = config.policy();
        let evidence = EvidenceStore::open(dir.join("evidence"))?;
        let store = SqliteStore::open(&dir.join("store.sqlite"))?;
        Ok(Self {
            root: root.to_path_buf(),
            work: root.to_path_buf(),
            config,
            checks,
            policy,
            evidence,
            store,
        })
    }

    /// Who is acting, as `RIS_AGENT` says: `person`, or a configured
    /// seat. Unset is refused, never taken to mean the person.
    pub fn actor(&self) -> Result<Actor, CliError> {
        match env::var("RIS_AGENT") {
            Ok(name) if name.trim() == PERSON => Ok(Actor::Person),
            Ok(name) if !name.trim().is_empty() => {
                let name = name.trim().to_owned();
                if !self.config.agents.contains_key(&name) {
                    return Err(CliError::UnknownAgent(name));
                }
                Ok(Actor::Agent(AgentName(name)))
            }
            _ => Err(CliError::NoActor(
                self.config.agents.keys().cloned().collect(),
            )),
        }
    }

    /// Scan free text from the command line under this project's policy.
    pub fn text(&self, field: &str, raw: &str) -> Result<Text, CliError> {
        Ok(Text::new(field, raw, &self.policy)?)
    }

    /// The events from `from` on, unverified: the caller verifies them
    /// against the head it holds.
    pub fn read_from(&self, from: u64) -> Result<Vec<Event>, CliError> {
        Ok(self.store.read_from(from)?)
    }

    /// Verify, fold and index the log.
    pub fn load(&self) -> Result<Loaded, CliError> {
        let log = self.store.read_all()?;
        ris_core::event::verify(&log).map_err(StoreError::Corrupt)?;
        let state = State::fold(&log).map_err(StoreError::Inconsistent)?;
        Ok(Loaded::new(log, state))
    }

    /// Apply a command and append its events, retrying when another
    /// writer gets in first. Returns the state after, and the events.
    pub fn run(&mut self, command: &Command) -> Result<(Loaded, Vec<Event>), CliError> {
        for _ in 0..20 {
            let loaded = self.load()?;
            let effects = apply(&loaded.state, command)?;
            let events = seal(&loaded.state, effects, now_millis(), fresh_salt);
            match self.store.append(&events) {
                Ok(()) => {
                    let after = self.load()?;
                    return Ok((after, events));
                }
                Err(StoreError::Conflict { .. }) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(CliError::Contended)
    }

    /// The chain head, for display.
    pub fn head(&self) -> Result<(u64, ris_core::ids::Hash), CliError> {
        Ok(self.store.head()?)
    }
}

/// Run git in `dir` and read one path from its output.
fn git_path(dir: &Path, args: &[&str]) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// The shell's clock: milliseconds since the Unix epoch.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sixteen bytes from the operating system's random source.
fn fresh_salt() -> Salt {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    Salt(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;

    /// Each role's refusal names its own next step: only an auditor is
    /// pointed at the review queue, only an advisor at what it may do.
    #[test]
    fn a_role_refusal_names_that_role_s_next_step() {
        for &role in Role::ALL {
            let text = CliError::RoleForbids {
                seat: "kite".into(),
                role,
                what: "claim an item",
            }
            .to_string();
            assert!(
                text.contains(&format!("is an {role}")) || text.contains(&format!("is a {role}")),
                "{text}"
            );
            assert!(
                !text.contains("an lead") && !text.contains("an builder"),
                "{text}"
            );
            assert_eq!(text.contains("ris audit"), role == Role::Auditor, "{text}");
            assert_eq!(
                text.contains("ris item add"),
                role == Role::Advisor,
                "{text}"
            );
        }
    }
}
