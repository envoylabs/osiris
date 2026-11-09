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
    /// Two writers kept colliding; the caller may retry.
    Contended,
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
                "RIS_AGENT={a} is not a seat in {DIR}/config.toml\n  next: add [agents.{a}] there, or unset RIS_AGENT to act as the person"
            ),
            Self::Contended => write!(f, "the log kept moving under this command; try again"),
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
    pub root: PathBuf,
    pub config: Config,
    pub checks: ChecksConfig,
    pub policy: Policy,
    pub evidence: EvidenceStore,
    store: SqliteStore,
}

impl Project {
    /// Walk up from `start` to the directory holding `.osiris/`.
    pub fn find(start: &Path) -> Result<Self, CliError> {
        let mut dir = Some(start);
        while let Some(d) = dir {
            if d.join(DIR).is_dir() {
                return Self::open(d);
            }
            dir = d.parent();
        }
        Err(CliError::NotAProject(start.to_path_buf()))
    }

    /// Create `.osiris/` in `root` from the templates.
    pub fn init(root: &Path) -> Result<Self, CliError> {
        let dir = root.join(DIR);
        if dir.exists() {
            return Err(CliError::AlreadyAProject(dir));
        }
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("config.toml"), Config::TEMPLATE)?;
        fs::write(dir.join("checks.toml"), ChecksConfig::TEMPLATE)?;
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
            config,
            checks,
            policy,
            evidence,
            store,
        })
    }

    /// Who is acting: the agent named by `RIS_AGENT`, or the person.
    pub fn actor(&self) -> Result<Actor, CliError> {
        match env::var("RIS_AGENT") {
            Ok(name) if !name.trim().is_empty() => {
                let name = name.trim().to_owned();
                if !self.config.agents.contains_key(&name) {
                    return Err(CliError::UnknownAgent(name));
                }
                Ok(Actor::Agent(AgentName(name)))
            }
            _ => Ok(Actor::Person),
        }
    }

    /// Scan free text from the command line under this project's policy.
    pub fn text(&self, field: &str, raw: &str) -> Result<Text, CliError> {
        Ok(Text::new(field, raw, &self.policy)?)
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
            let events = seal(&loaded.state, effects, fresh_salt);
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

/// Sixteen bytes from the operating system's random source.
fn fresh_salt() -> Salt {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    Salt(bytes)
}
