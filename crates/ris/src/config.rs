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

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("config.toml: {e}"))
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

# Agent seats. A name is a durable pseudonym for a seat, never a real
# person's name. Set RIS_AGENT=<name> in an agent's environment.
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
    fn a_denylisted_agent_name_is_refused() {
        let c = Config::parse(
            "denylist = [\"Example Person\"]\n[agents.\"example person\"]\nrole = \"lead\"\n",
        )
        .unwrap();
        assert!(c.check_agent_names().is_err());
    }
}
