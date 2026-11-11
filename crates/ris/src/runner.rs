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
        // Reading and `ris` only: everything it does goes through ris.
        Role::Advisor => {}
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
         RIS_AGENT is set to {seat} and `ris` is on your PATH: run `ris ...` \
         as it is, never with RIS_AGENT or a path in front, since the allow \
         rules match `ris ...`. Your rules file has the rules; follow them. \
         Learn the state from `ris inbox`, `ris ready` and `ris report`, run \
         first, and from no status document. "
    );
    let work = match role {
        Role::Lead | Role::Builder => {
            "Work the loop: read `ris inbox`, take the next item from `ris ready` \
             with `ris claim` (its output is your brief), do it, run `ris done`, \
             commit once with the Item and Agent trailers, then `ris merge`. \
             Repeat. Raise decisions and items as the rules say, and never \
             take a substantive decision. "
        }
        Role::Advisor => {
            "Work beside the person: read `ris summary` and `ris inbox`; add \
             what needs doing with `ris item add`, send what a seat should \
             know with `ris send`, and put judgements to the person with \
             `ris decision raise --substantive` and `ris decision explain`. \
             Never claim, finish, merge or decide, and never write to the tree. "
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
/// writing only where the seat works and where `ris` and `git` must,
/// and never to `protected`.
pub fn sandbox_settings(write: &[String], protected: &[String], network: &[String]) -> String {
    json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "filesystem": { "allowWrite": write, "denyWrite": protected },
            "network": { "allowedDomains": network },
        }
    })
    .to_string()
}

/// The rules files an agent tool loads from a directory, by name: what
/// osiris writes (`OSIRIS.md`, and `CLAUDE.local.md` importing it) and
/// what a project may use instead (`AGENTS.md`, `CLAUDE.md`, and
/// `.claude/CLAUDE.md`).
pub const RULES_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    crate::project::LOCAL_IMPORT_FILE,
    crate::project::RULES_FILE,
];

/// The files that set an agent's limits, which it must never write:
/// the configuration (its allowlist, sandbox and denylist), the
/// checks, and every rules file an agent loads, in the main checkout,
/// in the seat's own directory and inside `.osiris/`, whether or not it
/// exists, so a seat can neither edit the rules in force nor create a
/// file that would take precedence. The store and evidence beside them
/// stay writable; the chain shows any tampering with the store.
pub fn protected(root: &Path, rules: &Path, dir: &Path) -> Vec<PathBuf> {
    let osiris = root.join(crate::project::DIR);
    let mut out = vec![
        osiris.join("config.toml"),
        osiris.join("checks.toml"),
        osiris.join(crate::project::RULES_FILE),
        rules.to_path_buf(),
    ];
    for base in [root, dir] {
        out.extend(RULES_FILES.iter().map(|f| base.join(f)));
    }
    out.sort();
    out.dedup();
    out
}

/// Deny rules for the agent tool's own file tools, by absolute path.
/// Deny rules are checked before allow rules.
pub fn deny_edits(protected: &[PathBuf]) -> Vec<String> {
    protected
        .iter()
        .map(|p| format!("Edit(/{})", p.display()))
        .collect()
}

/// Claude Code's command line for an unattended run: print mode,
/// streamed JSON, refused rather than asked outside the allowlist.
pub fn claude_args(
    prompt: &str,
    allow: &[String],
    deny: &[String],
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
    if !deny.is_empty() {
        args.push("--disallowedTools".into());
        args.push(deny.join(","));
    }
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
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(dir)
        .env("RIS_AGENT", seat)
        .env("RIS_ROOT", root);
    if let Some(path) = path_with_ris(std::env::current_exe().ok().as_deref()) {
        cmd.env("PATH", path);
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).spawn()?;
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

/// The seat's PATH: the directory of the `ris` that started it, ahead
/// of the inherited PATH. A seat then runs the harness's own `ris` by
/// name, which the `Bash(ris *)` rule matches, rather than a binary it
/// may be rebuilding in its worktree. `None` when it cannot be formed.
fn path_with_ris(exe: Option<&Path>) -> Option<std::ffi::OsString> {
    let dir = exe?.parent()?.to_path_buf();
    let rest = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&rest))).ok()
}

/// The lines of `text` from byte offset `from`, and the offset to read
/// from next. A run file is read while it is written, so a last line
/// with no newline may be only part of a write: it is left for the next
/// read unless `to_end` says the writer has finished.
pub fn lines_from(text: &str, from: usize, to_end: bool) -> (Vec<&str>, usize) {
    let rest = text.get(from..).unwrap_or("");
    let take = if to_end {
        rest.len()
    } else {
        rest.rfind('\n').map_or(0, |n| n + 1)
    };
    (rest[..take].lines().collect(), from + take)
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

/// The file of a seat's latest run in `runs` (`<seat>-<n>.jsonl`, the
/// highest `n`), or `None` when it has not run.
pub fn latest_run(runs: &Path, seat: &str) -> Option<PathBuf> {
    let prefix = format!("{seat}-");
    std::fs::read_dir(runs)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n: u64 = name
                .strip_prefix(&prefix)?
                .strip_suffix(".jsonl")?
                .parse()
                .ok()?;
            Some((n, e.path()))
        })
        .max()
        .map(|(_, path)| path)
}

/// What a run last did, from its output: the last line of the last
/// record with anything to read (a tool it ran, or what it said), on
/// one line and cut to `width` characters. `None` before it has said
/// anything.
pub fn last_activity(text: &str, width: usize) -> Option<String> {
    let shown = text.lines().rev().find_map(render_line)?;
    let last = shown.lines().rev().find(|l| !l.trim().is_empty())?.trim();
    Some(if last.chars().count() > width {
        let cut: String = last.chars().take(width.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        last.to_owned()
    })
}

/// Where a seat's git data lives: the repository's common git
/// directory, and the seat's own entry under its `worktrees/` when the
/// seat works in a linked worktree (`None` in the main checkout).
pub struct GitDirs {
    pub common: PathBuf,
    pub own: Option<PathBuf>,
}

/// What a commit, a rebase and a merge write in the common git
/// directory, besides its `objects/`, `refs/` and `logs/`: the main
/// checkout's state files and their lock files, and automatic garbage
/// collection's own files. None of them is run or read as
/// configuration. `hooks/`, `config` and `info/` are not among them.
const GIT_STATE: &[&str] = &[
    "packed-refs",
    "index",
    "HEAD",
    "ORIG_HEAD",
    "FETCH_HEAD",
    "COMMIT_EDITMSG",
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "AUTO_MERGE",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "gc.pid",
    "gc.log",
];

/// Directories in the common git directory a writing seat needs whole.
const GIT_TREES: &[&str] = &[
    "objects",
    "refs",
    "logs",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
];

/// The paths a seat's agent must be able to write besides its own
/// directory: the shared store, and for a writing seat what git writes
/// to commit, rebase and merge: objects, refs, logs, the state files
/// and the seat's own worktree entry. Never the git directory whole,
/// whose `hooks/` and `config` run outside the sandbox the next time
/// git runs as the person.
pub fn writable(root: &Path, role: Role, git: Option<&GitDirs>) -> Vec<PathBuf> {
    let mut out = vec![root.join(crate::project::DIR)];
    if role.writes()
        && let Some(g) = git
    {
        out.extend(GIT_TREES.iter().map(|d| g.common.join(d)));
        for f in GIT_STATE {
            out.push(g.common.join(f));
            out.push(g.common.join(format!("{f}.lock")));
        }
        out.extend(g.own.clone());
    }
    out
}

/// What in the git directories a seat must never write, though it may
/// write around them: what git runs or reads as configuration (`hooks/`,
/// `config`, `config.worktree`, `info/`, submodules' git directories),
/// the links that say where a worktree's git directory is (its entry's
/// `commondir` and `gitdir`, and the `.git` file in the seat's own
/// directory), so a seat cannot point git at a directory of its own.
pub fn git_protected(git: &GitDirs, dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["hooks", "config", "config.worktree", "info", "modules"]
        .iter()
        .map(|f| git.common.join(f))
        .collect();
    if let Some(own) = &git.own {
        out.extend(
            ["config.worktree", "commondir", "gitdir"]
                .iter()
                .map(|f| own.join(f)),
        );
        out.push(dir.join(".git"));
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
            serde_json::from_str(&sandbox_settings(&["x".into()], &["y".into()], &[])).unwrap();
        assert_eq!(s["sandbox"]["filesystem"]["denyWrite"], json!(["y"]));
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
            &["Edit(//r/c)".into()],
            "{}",
            Some("m"),
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--disallowedTools" && w[1] == "Edit(//r/c)")
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

    /// The last activity is the last thing a run did, skipping
    /// bookkeeping records, and is cut to one short line.
    #[test]
    fn the_last_activity_is_the_last_thing_said_or_run() {
        let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"claiming I-3"},{"type":"tool_use","name":"Bash","input":{"command":"ris claim I-3"}}]}}"#;
        let user = r#"{"type":"user","message":{}}"#;
        let run = format!("{said}\n{user}\n");
        assert_eq!(
            last_activity(&run, 80).as_deref(),
            Some("→ Bash ris claim I-3")
        );
        assert_eq!(last_activity(&run, 8).as_deref(), Some("→ Bash …"));
        assert_eq!(last_activity(user, 80), None);
        assert_eq!(last_activity("", 80), None);
    }

    /// A seat's latest run is its highest-numbered run file; another
    /// seat's files, and names that only start the same, are not it.
    #[test]
    fn the_latest_run_is_the_highest_numbered() {
        let dir = std::env::temp_dir().join(format!("ris-latest-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(latest_run(&dir, "rook"), None);
        for f in [
            "rook-2.jsonl",
            "rook-10.jsonl",
            "rooky-99.jsonl",
            "heron-50.jsonl",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        assert_eq!(latest_run(&dir, "rook"), Some(dir.join("rook-10.jsonl")));
        assert_eq!(latest_run(&dir.join("absent"), "rook"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A line that reaches the file in two writes is read once, whole,
    /// after its second write; a last line without a newline is read
    /// only when the writer has finished.
    #[test]
    fn a_line_written_in_two_parts_is_read_once_whole() {
        let line = r#"{"type":"result","subtype":"success"}"#;
        let (half, rest) = line.split_at(10);
        let mut file = String::from("first\n");
        file.push_str(half);
        let (got, at) = lines_from(&file, 0, false);
        assert_eq!(got, ["first"]);
        assert_eq!(at, 6);
        file.push_str(rest);
        file.push('\n');
        let (got, at) = lines_from(&file, at, false);
        assert_eq!(got, [line]);
        assert_eq!(at, file.len());
        assert_eq!(lines_from(&file, at, false), (vec![], file.len()));
        // The writer stopped mid-line: the end is read as it is.
        file.push_str("cut");
        assert_eq!(lines_from(&file, at, false).0, Vec::<&str>::new());
        assert_eq!(lines_from(&file, at, true), (vec!["cut"], file.len()));
    }

    #[test]
    fn the_files_that_set_limits_are_denied_by_absolute_path() {
        let p = protected(
            Path::new("/r"),
            Path::new("/r/.osiris/OSIRIS.md"),
            Path::new("/r-rook"),
        );
        let d = deny_edits(&p);
        for want in [
            "Edit(//r/.osiris/config.toml)",
            "Edit(//r/.osiris/checks.toml)",
            "Edit(//r/.osiris/OSIRIS.md)",
            "Edit(//r-rook/CLAUDE.local.md)",
        ] {
            assert!(d.contains(&want.to_owned()), "{want} in {d:?}");
        }
        // Every rules file an agent loads, in both places, whatever the
        // rules path is and whether or not the files exist.
        let p = protected(
            Path::new("/r"),
            Path::new("/r/OSIRIS.md"),
            Path::new("/r-rook"),
        );
        for base in ["/r", "/r-rook"] {
            for f in RULES_FILES {
                let want = Path::new(base).join(f);
                assert!(p.contains(&want), "{} in {p:?}", want.display());
            }
        }
        assert!(p.contains(&PathBuf::from("/r/.osiris/OSIRIS.md")));
        let mut unique = p.clone();
        unique.dedup();
        assert_eq!(unique, p, "each path once");
    }

    #[test]
    fn each_role_is_told_when_to_stop() {
        for role in Role::ALL.iter().copied() {
            let p = prompt("rook", role);
            assert!(p.contains("RIS_AGENT is set to rook"), "{p}");
            // What the cold-start drill (crate::drill) holds a seat to.
            assert!(
                p.contains("`ris inbox`, `ris ready` and `ris report`, run first"),
                "{p}"
            );
            assert!(p.contains("refused because you are paused"), "{p}");
            assert!(!p.contains("person action"), "{p}");
        }
        assert!(prompt("wren", Role::Auditor).contains("Never write to the tree"));
        assert!(prompt("kite", Role::Advisor).contains("never write to the tree"));
    }

    #[test]
    fn an_advisor_reads_and_runs_ris_and_nothing_more() {
        let a = allowlist(Role::Advisor, &checks(), &[]);
        assert_eq!(a, READ.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        let g = GitDirs {
            common: PathBuf::from("/r/.git"),
            own: None,
        };
        let w = writable(Path::new("/r"), Role::Advisor, Some(&g));
        assert_eq!(w, vec![PathBuf::from("/r/.osiris")]);
        for role in Role::ALL.iter().copied() {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
    }

    #[test]
    fn a_writing_seat_writes_what_git_commits_need_and_never_hooks_or_config() {
        let g = GitDirs {
            common: PathBuf::from("/r/.git"),
            own: Some(PathBuf::from("/r/.git/worktrees/r-rook")),
        };
        let w = writable(Path::new("/r"), Role::Builder, Some(&g));
        for want in [
            "/r/.git/objects",
            "/r/.git/refs",
            "/r/.git/logs",
            "/r/.git/packed-refs.lock",
            "/r/.git/worktrees/r-rook",
        ] {
            assert!(w.contains(&PathBuf::from(want)), "{want} in {w:?}");
        }
        assert!(
            !w.contains(&g.common),
            "never the git directory whole: {w:?}"
        );
        for w in &w {
            assert!(
                !w.starts_with("/r/.git/hooks")
                    && !w.starts_with("/r/.git/config")
                    && !w.starts_with("/r/.git/info")
                    && !w.starts_with("/r/.git/worktrees/r-heron"),
                "{} is writable",
                w.display()
            );
        }
        let d = git_protected(&g, Path::new("/r-rook"));
        for want in [
            "/r/.git/hooks",
            "/r/.git/config",
            "/r/.git/info",
            "/r/.git/worktrees/r-rook/commondir",
            "/r/.git/worktrees/r-rook/gitdir",
            "/r/.git/worktrees/r-rook/config.worktree",
            "/r-rook/.git",
        ] {
            assert!(d.contains(&PathBuf::from(want)), "{want} in {d:?}");
        }
        // In the main checkout there is no worktree entry to protect.
        let main = GitDirs {
            common: PathBuf::from("/r/.git"),
            own: None,
        };
        assert!(!git_protected(&main, Path::new("/r")).contains(&PathBuf::from("/r/.git")));
    }
}
