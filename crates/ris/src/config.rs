//! Configuration: `.osiris/config.toml` and `.osiris/checks.toml`.
//!
//! `config.toml` names the agent seats and holds the local denylist. It
//! is local to the machine and is never exported; the denylist is
//! itself sensitive. `checks.toml` declares the project's checks: a
//! command and its pass condition.

use std::collections::BTreeMap;

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
}

/// What a seat does.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Lead,
    Builder,
    Auditor,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lead => "lead",
            Self::Builder => "builder",
            Self::Auditor => "auditor",
        })
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
        Ok(c)
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

# Agent seats. A name is a durable pseudonym for a seat, never a real
# person's name. Set RIS_AGENT=<name> in an agent's environment,
# and RIS_AGENT=person in the person's; unset is refused.
[agents.heron]
role = \"lead\"

[agents.wren]
role = \"auditor\"
";
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
    fn a_denylisted_agent_name_is_refused() {
        let c = Config::parse(
            "denylist = [\"Example Person\"]\n[agents.\"example person\"]\nrole = \"lead\"\n",
        )
        .unwrap();
        assert!(c.check_agent_names().is_err());
    }
}
