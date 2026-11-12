//! Configuration: `.osiris/config.toml` and `.osiris/checks.toml`.
//!
//! `config.toml` names the agent seats and holds the local denylist. It
//! is local to the machine and is never exported; the denylist is
//! itself sensitive. `checks.toml` declares the project's checks: a
//! command and its pass condition.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use ris_core::text::{Policy, scan};
use serde::{Deserialize, Serialize};

/// What `config.toml` holds. Unknown keys are refused, so a key in the
/// wrong place (such as a denylist written under a seat's table) is an
/// error, not a silent no-op.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Agent seats by durable name.
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    /// Terms that must never appear in state: the person's name, client
    /// names, the home directory. Case-insensitive.
    #[serde(default)]
    pub denylist: Vec<String>,
    /// The main line: the branch each phase's branch merges into at
    /// close. When the phase's work is on the main line itself, there
    /// is nothing to merge.
    #[serde(default = "default_main_line")]
    pub main_line: String,
    /// Whether `ris phase open` creates `phase/P-n` from the main line
    /// and `ris phase close` merges it back and deletes it. When off,
    /// phases leave branches alone and close merges whatever branch the
    /// main worktree has checked out, if it is not the main line.
    #[serde(default = "default_true")]
    pub phase_branches: bool,
    /// `ris phase open` refuses while the denylist is empty, since text
    /// is scanned as it is written and the log cannot be redacted. Set
    /// this to say that an empty denylist is deliberate.
    #[serde(default)]
    pub confirm_empty_denylist: bool,
    /// Whether `ris merge` requires `Item:` and `Agent:` trailers. When
    /// off, the item-to-commit mapping and authorship live only in the
    /// log, and commit messages follow the project's own convention.
    #[serde(default = "default_true")]
    pub trailers: bool,
    /// Whether `ris phase close` merges the phase branch into the main
    /// line. When off, close records the review and leaves the branch
    /// to be pushed and merged on the git server.
    #[serde(default = "default_true")]
    pub merge_on_close: bool,
    /// How `ris seat start` runs agents unattended.
    #[serde(default)]
    pub runner: RunnerConfig,
    /// Whether `ris phase open` adds an item to bring the docs up to
    /// date, which `ris phase close` waits for. On unless turned off.
    #[serde(default = "default_true")]
    pub docs_item: bool,
    /// Where a writing seat's worktree goes: a path template, relative
    /// to the main checkout unless absolute, in which `{repo}` is the
    /// main checkout's directory name and `{seat}` the seat's name. It
    /// must contain `{seat}`, so no two seats share a path.
    #[serde(default = "default_worktrees")]
    pub worktrees: String,
    /// How long a person action's check may run, in seconds, before it
    /// is stopped and counted as not passing yet. The checks run from
    /// the person's inbox, summary and up, so a check that hangs must
    /// not hang the person's command.
    #[serde(default = "default_action_check_seconds")]
    pub action_check_seconds: u64,
    /// The person's ris, rebuilt from the last merged commit and
    /// installed on their PATH (D-48). Off when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<crate::install::InstallConfig>,
    /// What `ris map` marks beside what it finds for itself.
    #[serde(default)]
    pub map: MapConfig,
}

/// `ris map`: the phase's change set drawn over the source code.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MapConfig {
    /// Path patterns for the files that are contracts and formats (a
    /// wire format's types, a schema, the document describing an
    /// interface), relative to the repository root. `*` matches within
    /// a path segment and `**` any number of segments. A file matching
    /// one is drawn as touching the outside, and is on the map even
    /// when it is not source code.
    #[serde(default)]
    pub contracts: Vec<String>,
    /// Path patterns for the files that are tests, in the same form. A
    /// test file is left off the map and counted in its totals instead:
    /// the behaviour lands in the source, and the tests only verify it.
    /// A file a contracts pattern names stays on the map.
    #[serde(default = "default_map_tests")]
    pub tests: Vec<String>,
}

impl Default for MapConfig {
    fn default() -> Self {
        MapConfig {
            contracts: Vec::new(),
            tests: default_map_tests(),
        }
    }
}

/// The default `[map] tests`: every file under a `tests` directory.
pub fn default_map_tests() -> Vec<String> {
    vec!["**/tests/**".to_owned()]
}

/// The default `worktrees`: beside the repository, as `<repo>-<seat>`.
pub const DEFAULT_WORKTREES: &str = "../{repo}-{seat}";

fn default_action_check_seconds() -> u64 {
    10
}

fn default_worktrees() -> String {
    DEFAULT_WORKTREES.to_owned()
}

/// The runner: which agent tool starts a seat's agent, and the line an
/// unattended agent may not cross. Each role has a built-in allowlist
/// (`crate::runner`); this adds to it and never removes from it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// The agent tool's program, found on PATH. Its adapter is chosen
    /// by this name; `claude` is the only adapter so far.
    #[serde(default = "default_program")]
    pub program: String,
    /// More tool permissions per role, in the agent tool's own syntax,
    /// for example `builder = ["Bash(cargo *)"]`.
    #[serde(default)]
    pub allow: BTreeMap<String, Vec<String>>,
    /// Network hosts the sandbox lets an agent reach. Empty: none.
    #[serde(default)]
    pub network: Vec<String>,
    /// More paths the sandbox lets an agent write, in the agent tool's
    /// syntax (`~/` is the home directory), for example a build tool's
    /// cache: `write = ["~/.cargo"]`.
    #[serde(default)]
    pub write: Vec<String>,
    /// How long, in seconds, a running seat's output may stay quiet
    /// before `ris status` marks it possibly stuck.
    #[serde(default = "default_quiet_after_seconds")]
    pub quiet_after_seconds: u64,
    /// Check, before `ris up` starts any seat, that the writing seats
    /// can run the project's checks: each check's program is on PATH,
    /// and the build tools it names may write their caches and reach
    /// their registries (`crate::preflight`). Off for a project whose
    /// build needs none of that, such as one with vendored dependencies.
    #[serde(default = "default_true")]
    pub preflight: bool,
}

fn default_quiet_after_seconds() -> u64 {
    600
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            program: default_program(),
            allow: BTreeMap::new(),
            network: Vec::new(),
            write: Vec::new(),
            quiet_after_seconds: default_quiet_after_seconds(),
            preflight: true,
        }
    }
}

fn default_program() -> String {
    "claude".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_main_line() -> String {
    "main".to_owned()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: BTreeMap::new(),
            denylist: Vec::new(),
            main_line: default_main_line(),
            phase_branches: true,
            confirm_empty_denylist: false,
            trailers: true,
            merge_on_close: true,
            runner: RunnerConfig::default(),
            docs_item: true,
            worktrees: default_worktrees(),
            action_check_seconds: default_action_check_seconds(),
            install: None,
            map: MapConfig::default(),
        }
    }
}

/// One agent seat.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub role: Role,
    /// The runner adapter that drives this seat, when runners exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// The model the runner asks for, in the agent tool's own naming.
    /// Unset: the tool's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether `ris up` starts this seat's agent and `ris down` stops
    /// it. `false` marks an attended seat, one a person runs
    /// interactively beside the unattended ones: up still gives it a
    /// worktree if it has none, and otherwise up and down leave it alone.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub start: bool,
}

fn is_true(b: &bool) -> bool {
    *b
}

/// What a seat does.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Lead,
    Builder,
    Auditor,
    /// Works beside the person: adds items, sends messages and raises
    /// decisions for the person, and nothing else.
    Advisor,
}

impl Role {
    pub const ALL: &[Role] = &[Self::Lead, Self::Builder, Self::Auditor, Self::Advisor];

    /// The role's name, as configuration and output spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Builder => "builder",
            Self::Auditor => "auditor",
            Self::Advisor => "advisor",
        }
    }

    /// The role by its name.
    pub fn parse(name: &str) -> Option<Role> {
        Self::ALL.iter().copied().find(|r| r.as_str() == name)
    }

    /// Whether the seat writes to the tree: takes, finishes and merges
    /// items, and so has a worktree and the git directory to commit to.
    pub fn writes(self) -> bool {
        matches!(self, Self::Lead | Self::Builder)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Self = toml::from_str(text).map_err(|e| format!("config.toml: {e}"))?;
        if c.agents.contains_key(crate::project::PERSON) {
            return Err(format!(
                "config.toml: `{}` names the person in RIS_AGENT and cannot be a seat",
                crate::project::PERSON
            ));
        }
        if !c.worktrees.contains("{seat}") {
            return Err(
                "config.toml: `worktrees` must contain {seat}, so no two seats share a path".into(),
            );
        }
        Ok(c)
    }

    /// Where `seat`'s worktree goes for the main checkout at `root`:
    /// `worktrees` filled in, joined to `root` unless absolute, with
    /// `.` and `..` resolved lexically.
    pub fn worktree_path(&self, root: &Path, seat: &str) -> PathBuf {
        let repo = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".into());
        let filled = self
            .worktrees
            .replace("{repo}", &repo)
            .replace("{seat}", seat);
        normalize(&root.join(filled))
    }

    /// The scan policy this configuration defines.
    pub fn policy(&self) -> Policy {
        Policy::new(&self.denylist)
    }

    /// Agent names appear in state and in exports, so they pass the same
    /// scan as any text, and must not be on the denylist.
    pub fn check_agent_names(&self) -> Result<(), String> {
        let policy = self.policy();
        for name in self.agents.keys() {
            if let Some(category) = scan(name, &policy) {
                return Err(format!(
                    "agent name in config.toml contains {}; use a pseudonym",
                    category.description()
                ));
            }
        }
        Ok(())
    }

    /// `text` (the file as it stands) with a seat added, comments and
    /// layout untouched: the seat is appended as its own table, after
    /// every top-level key. Refused when the name is not a plain
    /// lowercase word, is already a seat, was a seat once (`retired`:
    /// names in the log, since a name is never given to another seat),
    /// or fails the scan or the denylist.
    pub fn with_seat(
        text: &str,
        name: &str,
        role: Role,
        retired: &[String],
    ) -> Result<String, String> {
        let plain = name.len() <= 32
            && name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !plain {
            return Err(
                "a seat name is a lowercase word of letters, digits and hyphens, at most 32 long"
                    .into(),
            );
        }
        if name == crate::project::PERSON {
            return Err(format!(
                "`{name}` names the person in RIS_AGENT and cannot be a seat"
            ));
        }
        let current = Self::parse(text)?;
        if current.agents.contains_key(name) {
            return Err(format!("{name} is already a seat"));
        }
        if retired.iter().any(|r| r == name) {
            return Err(format!(
                "{name} was a seat before and is in the log; a name is never given to another seat"
            ));
        }
        let mut out = text.trim_end().to_owned();
        out.push_str(&format!("\n\n[agents.{name}]\nrole = \"{role}\"\n"));
        let parsed = Self::parse(&out)?;
        parsed.check_agent_names()?;
        Ok(out)
    }

    /// The template `ris init` writes.
    pub const TEMPLATE: &'static str = "\
# osiris configuration. Local to this machine; never exported.

# Terms that must never appear in harness state or commits: your name,
# client names, your home directory. Matching is case-insensitive.
# (Top-level keys go above the first [table]; below it, TOML reads them
# as part of that table.) `ris phase open` refuses while this is empty:
# fill it first, or set confirm_empty_denylist = true if empty is right.
denylist = []

# The branch each phase's branch merges into when the phase closes.
main_line = \"main\"

# Phase branches: `ris phase open` creates phase/P-n from main_line and
# switches the main worktree to it; `ris phase close` merges it back and
# deletes it. Set to false to manage branches yourself.
phase_branches = true

# Whether ris merge requires Item: and Agent: trailers in commit
# messages. Off, the log alone maps commits to items and authors.
trailers = true

# Whether ris phase close merges the phase branch into main_line. Off,
# close records the review and leaves the branch for a merge on the
# git server (a pull or merge request).
merge_on_close = true

# Where each writing seat's worktree goes: a path template, relative to this
# checkout unless absolute. {repo} is this checkout's directory name and
# {seat} the seat's name, which the template must contain.
worktrees = \"../{repo}-{seat}\"

# How seats' agents are run. `ris status` marks a running seat possibly
# stuck once its output has been quiet for quiet_after_seconds. Before
# ris up starts any seat, preflight checks that the writing seats can
# run the checks: each check's program is on PATH, and the build tools
# it names may write their caches (write) and reach their registries
# (network); it names any line missing. Set preflight = false if the
# build needs none of that.
[runner]
quiet_after_seconds = 600
preflight = true

# The person's ris, rebuilt from the last merged commit (which passed
# every check) and installed on your PATH, by `ris install` and on your
# next command after a merge. For a project that builds ris itself. The
# build runs in a checkout outside the repository (`dir`, by default in
# your cache directory). Uncomment to turn it on.
# [install]
# build = \"cargo build --release -p ris\"
# artifact = \"target/release/ris\"
# to = \"~/.local/bin/ris\"

# What `ris map` marks as contracts and formats, beside the files it
# finds touching the outside for itself: path patterns relative to the
# repository root, where * matches within a path segment and ** any
# number of segments. Uncomment and list your own. `tests` names the
# test files, which the map leaves out and counts instead; by default,
# every file under a tests directory.
# [map]
# contracts = [\"docs/api.md\", \"src/wire/**\", \"**/*.proto\"]
# tests = [\"**/tests/**\"]

# Agent seats. A name is a durable pseudonym for a seat, never a real
# person's name. Set RIS_AGENT=<name> in an agent's environment,
# and RIS_AGENT=person in the person's; unset is refused.
[agents.heron]
role = \"lead\"

[agents.wren]
role = \"auditor\"
";
}

/// `path` with `.` dropped and each `..` taking off the component
/// before it, without touching the filesystem (the path may not exist).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` relative to `from`, both absolute, as `../x` or `x/y`; `.`
/// when they are the same. Shown on screen in place of an absolute
/// path, which would carry a username.
pub fn relative(from: &Path, path: &Path) -> PathBuf {
    let from = normalize(from);
    let path = normalize(path);
    let a: Vec<_> = from.components().collect();
    let b: Vec<_> = path.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..a.len() {
        out.push("..");
    }
    for c in &b[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// What `checks.toml` holds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChecksConfig {
    #[serde(default)]
    pub checks: BTreeMap<String, CheckConfig>,
}

impl ChecksConfig {
    /// The checks an item is held to: the ones it names, or every
    /// declared check when it names none (`None`). `Some(&[])` is an
    /// item with no checks, which needs attached evidence to be done.
    pub fn for_item(&self, named: Option<&[String]>) -> Vec<ris_core::ids::CheckId> {
        match named {
            Some(names) => names.iter().cloned().map(ris_core::ids::CheckId).collect(),
            None => self
                .checks
                .keys()
                .cloned()
                .map(ris_core::ids::CheckId)
                .collect(),
        }
    }
}

/// One check: a shell command, passing on exit status zero.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CheckConfig {
    pub command: String,
}

impl ChecksConfig {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("checks.toml: {e}"))
    }

    /// The template `ris init` writes.
    pub const TEMPLATE: &'static str = "\
# Project checks: a command and its pass condition (exit status zero).
# Name them on items with `ris item add --check <name>`.

[checks.test]
command = \"cargo test --workspace\"
";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_templates_parse() {
        let c = Config::parse(Config::TEMPLATE).unwrap();
        assert_eq!(c.agents["heron"].role, Role::Lead);
        assert_eq!(c.agents["wren"].role, Role::Auditor);
        assert!(c.denylist.is_empty());
        c.check_agent_names().unwrap();
        assert_eq!(c.install, None, "installing is off until configured");
        let on = Config::parse(&Config::TEMPLATE.replace("# [install]", "[install]").replace(
            "\n# build = \"cargo build --release -p ris\"\n# artifact = \"target/release/ris\"\n# to =",
            "\nbuild = \"cargo build --release -p ris\"\nartifact = \"target/release/ris\"\nto =",
        ))
        .unwrap();
        let install = on
            .install
            .expect("the template's example, uncommented, parses");
        assert_eq!(install.artifact, "target/release/ris");
        assert_eq!(on.agents.len(), 2, "the seats below stay seats");
        let k = ChecksConfig::parse(ChecksConfig::TEMPLATE).unwrap();
        assert_eq!(k.checks["test"].command, "cargo test --workspace");
    }

    #[test]
    fn a_denylist_filled_in_the_template_takes_effect() {
        let c = Config::parse(&Config::TEMPLATE.replace("denylist = []", "denylist = [\"acme\"]"))
            .unwrap();
        assert_eq!(c.policy().denylist_len(), 1);
    }

    #[test]
    fn a_key_in_the_wrong_table_is_an_error() {
        let misplaced = "[agents.wren]\nrole = \"auditor\"\ndenylist = [\"acme\"]\n";
        assert!(Config::parse(misplaced).is_err());
    }

    #[test]
    fn a_seat_is_appended_with_comments_kept_and_bad_names_refused() {
        let out = Config::with_seat(Config::TEMPLATE, "rook", Role::Builder, &[]).unwrap();
        assert!(
            out.starts_with(Config::TEMPLATE.trim_end()),
            "nothing above is rewritten"
        );
        let c = Config::parse(&out).unwrap();
        assert_eq!(c.agents["rook"].role, Role::Builder);
        assert_eq!(c.agents.len(), 3);

        let refused = |name: &str, retired: &[String]| {
            Config::with_seat(Config::TEMPLATE, name, Role::Lead, retired).is_err()
        };
        assert!(refused("heron", &[]), "taken");
        assert!(refused("kite", &["kite".to_owned()]), "retired");
        for bad in ["", "Rook", "two words", "x]\ndenylist = []\n[y", "9lives"] {
            assert!(refused(bad, &[]), "{bad:?}");
        }
        let with_denylist =
            Config::TEMPLATE.replace("denylist = []", "denylist = [\"examplename\"]");
        assert!(
            Config::with_seat(&with_denylist, "examplename", Role::Lead, &[]).is_err(),
            "denylisted"
        );
    }

    #[test]
    fn worktrees_default_beside_the_repository_and_can_be_placed() {
        // A root assembled from pieces: a literal absolute path in the
        // tree is refused by the exit scan.
        let root: PathBuf = [std::path::MAIN_SEPARATOR_STR, "w", "proj"]
            .iter()
            .collect();
        let base = root.parent().unwrap();
        let c = Config::parse(Config::TEMPLATE).unwrap();
        assert_eq!(c.worktrees, DEFAULT_WORKTREES);
        assert_eq!(c.worktree_path(&root, "rook"), base.join("proj-rook"));
        assert_eq!(
            Config::default().worktree_path(&root, "rook"),
            base.join("proj-rook"),
            "unset is the default"
        );

        let inside = Config::parse("worktrees = \".seats/{seat}\"\n").unwrap();
        assert_eq!(
            inside.worktree_path(&root, "kite"),
            root.join(".seats").join("kite")
        );
        let elsewhere = Config::parse("worktrees = \"../trees/{repo}/./{seat}\"\n").unwrap();
        assert_eq!(
            elsewhere.worktree_path(&root, "kite"),
            base.join("trees").join("proj").join("kite")
        );
        assert!(
            Config::parse("worktrees = \"../{repo}-shared\"\n").is_err(),
            "every seat would share one path"
        );
    }

    #[test]
    fn a_path_is_shown_relative_without_its_prefix() {
        let root: PathBuf = [std::path::MAIN_SEPARATOR_STR, "w", "proj"]
            .iter()
            .collect();
        let base = root.parent().unwrap();
        assert_eq!(
            relative(&root, &base.join("proj-rook")),
            Path::new("../proj-rook")
        );
        assert_eq!(
            relative(&root, &root.join(".seats/kite")),
            Path::new(".seats/kite")
        );
        assert_eq!(relative(&root, &root), Path::new("."));
        assert_eq!(
            relative(&root, &base.join("a/../trees/x")),
            Path::new("../trees/x")
        );
    }

    #[test]
    fn a_denylisted_agent_name_is_refused() {
        let c = Config::parse(
            "denylist = [\"Example Person\"]\n[agents.\"example person\"]\nrole = \"lead\"\n",
        )
        .unwrap();
        assert!(c.check_agent_names().is_err());
    }
}
