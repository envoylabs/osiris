//! The runner: starts a seat's agent unattended, in the seat's own
//! directory, with its name set, and records when it starts and stops.
//!
//! What an unattended agent may do is drawn here, per role, and
//! rendered for the agent tool (D-32): the allowlist below plus the
//! project's own check commands and whatever `[runner.allow]` adds;
//! anything else is refused rather than asked, since nobody is there to
//! answer. The tool's sandbox confines the shell to the seat's
//! directory, plus the two places `ris` and `git` must write from a
//! worktree: the shared `.osiris/` and the repository's git directory.
//!
//! One adapter so far, for Claude Code (D-27). An adapter turns the
//! same inputs (prompt, allowlist, writable paths, network hosts, model)
//! into its tool's command line; adding a tool is adding one.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

use crate::config::{ChecksConfig, Role};

/// Tool permissions every role has: reading, and `ris` itself.
const READ: &[&str] = &["Read", "Grep", "Glob", "Bash(ris *)"];
/// What writing seats add: edits, and git.
const WRITE: &[&str] = &["Edit", "Write", "Bash(git *)"];
/// What an auditor adds: reading history, never writing it.
const AUDIT: &[&str] = &[
    "Bash(git log *)",
    "Bash(git diff *)",
    "Bash(git show *)",
    "Bash(git status)",
];

/// The tool permissions for a role: the built-in list, the project's
/// check commands for a writing seat, then the configured additions.
pub fn allowlist(role: Role, checks: &ChecksConfig, extra: &[String]) -> Vec<String> {
    let mut out: Vec<String> = READ.iter().map(|s| (*s).to_owned()).collect();
    match role {
        Role::Lead | Role::Builder => {
            out.extend(WRITE.iter().map(|s| (*s).to_owned()));
            out.extend(
                checks
                    .checks
                    .values()
                    .map(|c| format!("Bash({})", c.command)),
            );
        }
        Role::Auditor => out.extend(AUDIT.iter().map(|s| (*s).to_owned())),
    }
    for e in extra {
        if !out.contains(e) {
            out.push(e.clone());
        }
    }
    out
}

/// What the agent is told when it starts. Its rules arrive through its
/// rules file; this is only the loop for its role and when to stop.
pub fn prompt(seat: &str, role: Role) -> String {
    let common = format!(
        "You are the {role} seat {seat} in a project directed with osiris. \
         RIS_AGENT is set to {seat}; never change it. Your rules file has the \
         rules; follow them. "
    );
    let work = match role {
        Role::Lead | Role::Builder => {
            "Work the loop: read `ris inbox`, take the next item from `ris ready` \
             with `ris claim` (its output is your brief), do it, run `ris done`, \
             commit once with the Item and Agent trailers, then `ris merge`. \
             Repeat. Raise decisions and person actions as the rules say, and \
             never take a substantive decision. "
        }
        Role::Auditor => {
            "Work through `ris audit`: for each item, read its diff and its \
             evidence against its goal, the decisions that touch it and the \
             rules; send each finding to its author with `ris send <seat> \
             --kind finding --about <item>`; then record the review with \
             `ris audit <item>`. Never write to the tree. "
        }
    };
    format!(
        "{common}{work}Stop when there is nothing left for you, or when a \
         command is refused because you are paused. If a command is refused as \
         not allowed, do not work around it: say what you needed, and stop."
    )
}

/// The agent tool's sandbox settings, as JSON: on, with no way out,
/// writing only where the seat works and where `ris` and `git` must.
pub fn sandbox_settings(write: &[String], network: &[String]) -> String {
    json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "filesystem": { "allowWrite": write },
            "network": { "allowedDomains": network },
        }
    })
    .to_string()
}

/// Claude Code's command line for an unattended run: print mode,
/// streamed JSON, refused rather than asked outside the allowlist.
pub fn claude_args(
    prompt: &str,
    allow: &[String],
    settings: &str,
    model: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--allowedTools",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push(allow.join(","));
    args.push("--settings".into());
    args.push(settings.to_owned());
    if let Some(m) = model {
        args.push("--model".into());
        args.push(m.to_owned());
    }
    args
}

/// Run `program` with `args` in `dir` as `seat`, copying each line it
/// prints to `log` and, unless `quiet`, to stdout. The exit status, or
/// `None` when it was ended by a signal.
pub fn run(
    program: &str,
    args: &[String],
    dir: &Path,
    seat: &str,
    root: &Path,
    log: &Path,
    quiet: bool,
) -> std::io::Result<Option<i32>> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(log)?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", seat)
        .env("RIS_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let out = child.stdout.take().expect("piped");
    let stdout = std::io::stdout();
    for line in BufReader::new(out).lines() {
        let line = line?;
        writeln!(file, "{line}")?;
        if !quiet {
            // A closed reader of our output does not stop the agent.
            let _ = writeln!(stdout.lock(), "{line}");
        }
    }
    Ok(child.wait()?.code())
}

/// One line of a run's output as text: the agent tool's stream JSON
/// turned into what was said and which tool was used, anything else
/// shown as it is; `None` for bookkeeping lines with nothing to read.
pub fn render_line(line: &str) -> Option<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Some(line.to_owned());
    };
    match v["type"].as_str() {
        Some("assistant") => {
            let parts: Vec<String> = v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| match c["type"].as_str() {
                    Some("text") => c["text"].as_str().map(str::to_owned),
                    Some("tool_use") => Some(format!(
                        "→ {} {}",
                        c["name"].as_str().unwrap_or("tool"),
                        c["input"]["command"]
                            .as_str()
                            .or(c["input"]["file_path"].as_str())
                            .unwrap_or("")
                    )),
                    _ => None,
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Some("result") => Some(format!(
            "== {} ==",
            v["subtype"].as_str().unwrap_or("result")
        )),
        Some("user" | "system") => None,
        _ => Some(line.to_owned()),
    }
}

/// The paths a seat's agent must be able to write besides its own
/// directory: the shared store, and the repository's git directory
/// for a writing seat (a worktree's commits are stored there).
pub fn writable(root: &Path, role: Role, git_dir: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out = vec![root.join(crate::project::DIR)];
    if role != Role::Auditor
        && let Some(g) = git_dir
    {
        out.push(g);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks() -> ChecksConfig {
        ChecksConfig::parse("[checks.test]\ncommand = \"cargo test\"\n").unwrap()
    }

    #[test]
    fn an_auditor_cannot_edit_and_a_builder_can_run_its_checks() {
        let a = allowlist(Role::Auditor, &checks(), &[]);
        assert!(a.contains(&"Bash(ris *)".to_owned()));
        assert!(
            !a.iter()
                .any(|t| t == "Edit" || t == "Write" || t == "Bash(git *)")
        );
        assert!(!a.contains(&"Bash(cargo test)".to_owned()));
        let b = allowlist(Role::Builder, &checks(), &["Bash(cargo *)".to_owned()]);
        for t in [
            "Edit",
            "Write",
            "Bash(git *)",
            "Bash(cargo test)",
            "Bash(cargo *)",
        ] {
            assert!(b.contains(&t.to_owned()), "{t}");
        }
        // Additions add; they never duplicate.
        let again = allowlist(Role::Builder, &checks(), &["Edit".to_owned()]);
        assert_eq!(again.iter().filter(|t| *t == "Edit").count(), 1);
    }

    #[test]
    fn the_sandbox_has_no_way_out() {
        let s: serde_json::Value =
            serde_json::from_str(&sandbox_settings(&["x".into()], &[])).unwrap();
        assert_eq!(s["sandbox"]["enabled"], true);
        assert_eq!(s["sandbox"]["failIfUnavailable"], true);
        assert_eq!(s["sandbox"]["allowUnsandboxedCommands"], false);
        assert_eq!(s["sandbox"]["network"]["allowedDomains"], json!([]));
    }

    #[test]
    fn claude_is_refused_rather_than_asked() {
        let args = claude_args(
            "go",
            &["Read".into(), "Bash(ris *)".into()],
            "{}",
            Some("m"),
        );
        let has = |a: &str, b: &str| args.windows(2).any(|w| w[0] == a && w[1] == b);
        assert!(has("-p", "go"));
        assert!(has("--permission-mode", "dontAsk"));
        assert!(has("--permission-prompts", "none"));
        assert!(has("--allowedTools", "Read,Bash(ris *)"));
        assert!(has("--settings", "{}"));
        assert!(has("--model", "m"));
        assert!(!args.iter().any(|a| a.contains("skip-permissions")));
    }

    #[test]
    fn a_stream_reads_as_what_was_said_and_done() {
        let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"claiming I-3"},{"type":"tool_use","name":"Bash","input":{"command":"ris claim I-3"}}]}}"#;
        assert_eq!(
            render_line(said).unwrap(),
            "claiming I-3\n→ Bash ris claim I-3"
        );
        assert_eq!(render_line(r#"{"type":"system","subtype":"init"}"#), None);
        assert_eq!(
            render_line(r#"{"type":"result","subtype":"success"}"#).unwrap(),
            "== success =="
        );
        assert_eq!(render_line("plain text").unwrap(), "plain text");
    }

    #[test]
    fn each_role_is_told_when_to_stop() {
        for role in [Role::Lead, Role::Builder, Role::Auditor] {
            let p = prompt("rook", role);
            assert!(p.contains("RIS_AGENT is set to rook"), "{p}");
            assert!(p.contains("refused because you are paused"), "{p}");
        }
        assert!(prompt("wren", Role::Auditor).contains("Never write to the tree"));
    }
}
