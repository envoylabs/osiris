//! The preflight: before `ris up` starts any seat, whether the writing
//! seats can do the phase's work, read from the configuration.
//!
//! A seat runs the project's checks inside the agent tool's sandbox,
//! which lets it write only its own directory, the shared store and
//! what `[runner] write` adds, and reach only the hosts in `[runner]
//! network`. A build tool that writes a cache in the home directory, or
//! fetches dependencies from a registry, fails there unless the
//! configuration allows it. The preflight finds the build tools each
//! check names, from a table of known ones, and names each path and
//! host the configuration does not allow, with the line that allows
//! it. It also finds each check's program on PATH, which a seat
//! inherits.
//!
//! This reads the configuration; it does not run the checks in a copy
//! of the sandbox, which is the agent tool's and is not reproduced
//! here. A tool not in the table is not checked. `[runner] preflight =
//! false` turns it off for a build that needs none of it.

use std::path::{Path, PathBuf};

use crate::config::{ChecksConfig, RunnerConfig};

/// A build tool a check may run: the programs that name it, the caches
/// it writes and the hosts it fetches dependencies from, at their
/// default locations.
struct Tool {
    name: &'static str,
    programs: &'static [&'static str],
    write: &'static [&'static str],
    network: &'static [&'static str],
}

/// Known build tools, at their defaults. Each tool's cache location is
/// its documented default with no environment override: `CARGO_HOME`,
/// `npm_config_cache`, `PIP_CACHE_DIR`, `UV_CACHE_DIR`, `GOMODCACHE`
/// and `GOCACHE` move them.
fn tools() -> Vec<Tool> {
    let mac = cfg!(target_os = "macos");
    vec![
        Tool {
            name: "cargo",
            programs: &["cargo"],
            write: &["~/.cargo/registry", "~/.cargo/git"],
            network: &["index.crates.io", "static.crates.io"],
        },
        Tool {
            name: "npm",
            programs: &["npm", "npx"],
            write: &["~/.npm"],
            network: &["registry.npmjs.org"],
        },
        Tool {
            name: "pip",
            programs: &["pip", "pip3"],
            write: if mac {
                &["~/Library/Caches/pip"]
            } else {
                &["~/.cache/pip"]
            },
            network: &["pypi.org", "files.pythonhosted.org"],
        },
        Tool {
            name: "uv",
            programs: &["uv"],
            write: &["~/.cache/uv"],
            network: &["pypi.org", "files.pythonhosted.org"],
        },
        Tool {
            name: "go",
            programs: &["go"],
            write: if mac {
                &["~/go/pkg/mod", "~/Library/Caches/go-build"]
            } else {
                &["~/go/pkg/mod", "~/.cache/go-build"]
            },
            network: &["proxy.golang.org", "sum.golang.org"],
        },
    ]
}

/// One thing a writing seat would lack. In `--json` output it is an
/// object whose `kind` is `program`, `write` or `network`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Missing {
    /// A check's program is not on the PATH a seat inherits.
    Program { check: String, program: String },
    /// A build tool's cache that `[runner] write` does not cover.
    Write { tool: String, path: String },
    /// A registry host that `[runner] network` does not allow.
    Network { tool: String, host: String },
}

/// What the writing seats would lack, in check order, each once.
pub fn check(checks: &ChecksConfig, runner: &RunnerConfig, path: Option<&str>) -> Vec<Missing> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut out = Vec::new();
    let tools = tools();
    for (name, c) in &checks.checks {
        let words = programs_in(&c.command);
        if let Some(first) = words.iter().find(|w| !BUILTINS.contains(&w.as_str()))
            && !on_path(first, path)
        {
            push(
                &mut out,
                Missing::Program {
                    check: name.clone(),
                    program: first.clone(),
                },
            );
        }
        for t in tools
            .iter()
            .filter(|t| words.iter().any(|w| t.programs.contains(&w.as_str())))
        {
            for w in t.write {
                if !runner
                    .write
                    .iter()
                    .any(|allowed| covers(allowed, w, home.as_deref()))
                {
                    push(
                        &mut out,
                        Missing::Write {
                            tool: t.name.into(),
                            path: (*w).into(),
                        },
                    );
                }
            }
            for h in t.network {
                if !runner.network.iter().any(|allowed| allows(allowed, h)) {
                    push(
                        &mut out,
                        Missing::Network {
                            tool: t.name.into(),
                            host: (*h).into(),
                        },
                    );
                }
            }
        }
    }
    out
}

/// Shell builtins a check may start with, which are not looked for on
/// PATH.
const BUILTINS: &[&str] = &[
    "cd", "export", "set", "unset", "source", ".", "exec", "eval", "test", "[", "true", "false",
    "echo", "command", "exit", ":",
];

fn push(out: &mut Vec<Missing>, m: Missing) {
    if !out.contains(&m) {
        out.push(m);
    }
}

/// The programs a check's command runs: the first word of each simple
/// command between `&&`, `||`, `;` and `|`, past any `VAR=value`
/// assignments, by its file name. The first that is not a shell builtin
/// is the check's own program.
fn programs_in(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let spaced = command
        .replace("&&", ";")
        .replace("||", ";")
        .replace('|', ";");
    for part in spaced.split(';') {
        if let Some(w) = part
            .split_whitespace()
            .find(|w| !(w.contains('=') && !w.starts_with('=')))
        {
            let name = Path::new(w)
                .file_name()
                .map_or(w.to_owned(), |n| n.to_string_lossy().into_owned());
            out.push(name);
        }
    }
    out
}

/// Whether `program` is found: a path that exists, or a name in one of
/// the directories of `path` (PATH as a seat inherits it).
fn on_path(program: &str, path: Option<&str>) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(path).any(|d| d.join(program).is_file())
}

/// Whether a `[runner] write` entry covers `need`: the same path or a
/// directory above it, `~/` being the home directory either way, and a
/// trailing `/**` or `/*` meaning the directory itself.
fn covers(allowed: &str, need: &str, home: Option<&Path>) -> bool {
    let expand = |s: &str| -> Option<PathBuf> {
        let s = s
            .trim_end_matches("/**")
            .trim_end_matches("/*")
            .trim_end_matches('/');
        match s.strip_prefix("~/") {
            Some(rest) => Some(home?.join(rest)),
            None if s == "~" => home.map(Path::to_path_buf),
            None => Some(PathBuf::from(s)),
        }
    };
    // Compared as written when there is no home directory to expand.
    match (expand(allowed), expand(need)) {
        (Some(a), Some(n)) => n.starts_with(a),
        _ => Path::new(need).starts_with(allowed.trim_end_matches("/**")),
    }
}

/// Whether a `[runner] network` entry allows `host`: the same host, or
/// a `*.domain` wildcard over it.
fn allows(allowed: &str, host: &str) -> bool {
    match allowed.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.')),
        None => allowed == host,
    }
}

/// The preflight's report: each missing thing on a line, then the
/// configuration lines that allow them, and how to turn it off.
pub fn render(missing: &[Missing]) -> String {
    let mut s = String::from("preflight: the writing seats cannot run this project's checks yet\n");
    let (mut write, mut network) = (Vec::new(), Vec::new());
    for m in missing {
        match m {
            Missing::Program { check, program } => s.push_str(&format!(
                "  check `{check}` runs `{program}`, which is not on PATH; install it\n"
            )),
            Missing::Write { tool, path } => {
                s.push_str(&format!(
                    "  {tool} writes {path}, which [runner] write does not allow\n"
                ));
                write.push(format!("\"{path}\""));
            }
            Missing::Network { tool, host } => {
                s.push_str(&format!(
                    "  {tool} fetches from {host}, which [runner] network does not allow\n"
                ));
                network.push(format!("\"{host}\""));
            }
        }
    }
    if !write.is_empty() || !network.is_empty() {
        s.push_str("  next: add to [runner] in .osiris/config.toml, beside what is there:\n");
        if !write.is_empty() {
            s.push_str(&format!("    write = [{}]\n", write.join(", ")));
        }
        if !network.is_empty() {
            s.push_str(&format!("    network = [{}]\n", network.join(", ")));
        }
    }
    s.push_str("  then `ris up` again; or set preflight = false under [runner] if the build needs none of it");
    s
}

#[cfg(test)]
#[path = "tests/preflight.rs"]
mod tests;
