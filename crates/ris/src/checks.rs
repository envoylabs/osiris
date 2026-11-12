//! Running checks and reading commits: the shell's side of `done` and
//! `merge`.
//!
//! A check is a shell command run in the project root; it passes on
//! exit status zero. Its combined output, with a header naming the
//! command and the status, is stored as evidence, and the log records
//! the run by evidence id. A commit's message must carry exactly one
//! `Item: I-n` trailer naming the item it merges.

use std::path::Path;
use std::process::Command;

use ris_core::ids::{EvidenceId, ItemId};

use crate::config::CheckConfig;
use crate::evidence::EvidenceStore;
use crate::project::CliError;

/// One check's run.
pub struct CheckOutcome {
    pub name: String,
    pub passed: bool,
    pub evidence: EvidenceId,
}

/// Run `check` in `root`, store its output, and say whether it passed.
pub fn run_check(
    root: &Path,
    name: &str,
    check: &CheckConfig,
    evidence: &EvidenceStore,
) -> Result<CheckOutcome, CliError> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(&check.command)
        .current_dir(root)
        .output()?;
    let status = out
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_owned());
    let mut blob = format!(
        "check: {name}\ncommand: {}\nexit: {status}\n\n",
        check.command
    )
    .into_bytes();
    blob.extend_from_slice(&out.stdout);
    if !out.stderr.is_empty() {
        blob.extend_from_slice(b"\n--- stderr ---\n");
        blob.extend_from_slice(&out.stderr);
    }
    let id = evidence.put(&blob)?;
    Ok(CheckOutcome {
        name: name.to_owned(),
        passed: out.status.success(),
        evidence: id,
    })
}

/// Run `check` in `dir` without recording anything: whether it passed,
/// and its combined output.
pub fn run_check_quietly(dir: &Path, check: &CheckConfig) -> Result<(bool, String), CliError> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(&check.command)
        .current_dir(dir)
        .output()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// How a command run under a time limit ended.
#[derive(Debug)]
pub enum Limited {
    /// It ended within the limit: whether it exited zero, and its
    /// combined output.
    Finished(bool, Vec<u8>),
    /// It ran past the limit and was stopped. `kill_fault` is what went
    /// wrong when stopping its process group, if anything did: with a
    /// fault, what the command started may still be running.
    TimedOut { kill_fault: Option<String> },
}

/// Send KILL to every process in the group `pgid`, with the `kill`
/// system call itself: the `kill` programs differ between systems in
/// how they read a group's negative id, and a program that misreads it
/// can exit zero having signalled something else. `None` when nothing
/// of the group is left running; otherwise why it may be.
fn kill_group(pgid: rustix::process::Pid) -> Option<String> {
    fault_of(
        pgid,
        rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL),
    )
}

/// What a failed kill of the group `pgid` means. No such group is not a
/// fault: every process in it has already ended, so nothing is left to
/// stop. Any other error leaves the group possibly running.
fn fault_of(pgid: rustix::process::Pid, sent: rustix::io::Result<()>) -> Option<String> {
    match sent {
        Ok(()) | Err(rustix::io::Errno::SRCH) => None,
        Err(e) => Some(format!("killing process group {pgid} failed: {e}")),
    }
}

/// Run `command` with `sh -c` in `dir`, with no input, for at most
/// `limit`. When it runs past the limit it is killed with everything it
/// started (it runs in a process group of its own), and what it printed
/// is dropped.
pub fn run_limited(
    dir: &Path,
    command: &str,
    limit: std::time::Duration,
) -> Result<Limited, CliError> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let mut child = Command::new("sh")
        .args(["-c", command])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    // Read both pipes as the command runs, so a command that prints a
    // lot does not stall on a full pipe and look like a hang.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let Some(status) = status else {
        // The whole group, so a child the shell started (which holds
        // the pipes open) goes too.
        let kill_fault = kill_group(rustix::process::Pid::from_child(&child));
        let _ = child.kill();
        let _ = child.wait();
        return Ok(Limited::TimedOut { kill_fault });
    };
    let mut bytes = out.join().unwrap_or_default();
    bytes.extend(err.join().unwrap_or_default());
    Ok(Limited::Finished(status.success(), bytes))
}

/// `git rev-parse HEAD` in `root`.
pub fn git_head(root: &Path) -> Result<String, CliError> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        return Err(CliError::Other(
            "git rev-parse HEAD failed; pass --commit <hash>".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The full message of `commit`.
pub fn commit_message(root: &Path, commit: &str) -> Result<String, CliError> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%B", commit])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        return Err(CliError::Other(format!(
            "git does not know commit {commit}\n  next: commit the work first, with `Item: <id>` as a trailer"
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The items named by `Item:` trailers in a commit message. A trailer
/// is a line `Item: I-n`, as git's own trailers are written.
pub fn item_trailers(message: &str) -> Vec<Result<ItemId, String>> {
    message
        .lines()
        .filter_map(|line| line.strip_prefix("Item:"))
        .map(|rest| {
            let rest = rest.trim();
            rest.parse::<ItemId>().map_err(|_| rest.to_owned())
        })
        .collect()
}

/// Check that `message` names exactly `item`, and nothing else.
pub fn require_trailer(message: &str, item: ItemId, commit: &str) -> Result<(), CliError> {
    let short = &commit[..commit.len().min(12)];
    let trailers = item_trailers(message);
    match trailers.as_slice() {
        [Ok(found)] if *found == item => Ok(()),
        [] => Err(CliError::Other(format!(
            "commit {short} has no `Item:` trailer\n  next: amend or recommit with `Item: {item}` as a trailer"
        ))),
        [Ok(found)] => Err(CliError::Other(format!(
            "commit {short} names {found}, not {item}\n  next: merge {found}, or recommit with `Item: {item}`"
        ))),
        [Err(bad)] => Err(CliError::Other(format!(
            "commit {short} has an `Item:` trailer that is not an item id ({bad})\n  next: use `Item: {item}`"
        ))),
        _ => Err(CliError::Other(format!(
            "commit {short} names more than one item; one commit merges one item\n  next: split the commit, or recommit with only `Item: {item}`"
        ))),
    }
}

/// Check that `message` carries exactly one `Agent:` trailer and that
/// it names `agent`, the seat that did the item. This is how a commit
/// records co-authorship: by the agent's alias, with no address and no
/// model, so the message scan needs no exceptions.
pub fn require_agent_trailer(message: &str, agent: &str, commit: &str) -> Result<(), CliError> {
    let short = &commit[..commit.len().min(12)];
    let found: Vec<&str> = message
        .lines()
        .filter_map(|l| l.strip_prefix("Agent:"))
        .map(str::trim)
        .collect();
    match found.as_slice() {
        [one] if *one == agent => Ok(()),
        [] => Err(CliError::Other(format!(
            "commit {short} has no `Agent:` trailer\n  next: amend with `Agent: {agent}` beside the `Item:` trailer"
        ))),
        [one] => Err(CliError::Other(format!(
            "commit {short} names agent {one}, but the item was done by {agent}\n  next: amend with `Agent: {agent}`"
        ))),
        _ => Err(CliError::Other(format!(
            "commit {short} names more than one agent; one item has one author\n  next: amend with only `Agent: {agent}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No group has this id: it is above the largest pid Linux or macOS
    // gives out, so a kill that did act on it would reach nothing.
    fn no_group() -> rustix::process::Pid {
        rustix::process::Pid::from_raw(0x7fff_fff0).unwrap()
    }

    #[test]
    fn a_group_that_is_not_there_is_not_a_fault() {
        assert_eq!(kill_group(no_group()), None);
    }

    #[test]
    fn a_kill_that_is_refused_is_reported_with_the_group_and_the_reason() {
        // The error is given, not provoked: a kill the system refuses
        // needs a group owned by someone else, and none is safe to aim
        // at.
        let fault = fault_of(no_group(), Err(rustix::io::Errno::PERM)).expect("a fault");
        assert!(
            fault.starts_with("killing process group 2147483632 failed: "),
            "{fault}"
        );
        assert_eq!(fault_of(no_group(), Ok(())), None);
    }

    /// The whole group goes, not only the shell: a background child the
    /// shell started, which would write a file if left to run, never
    /// writes it.
    #[test]
    fn a_command_stopped_at_its_limit_takes_what_it_started_with_it() {
        let dir = std::env::temp_dir().join(format!("ris-checks-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let limit = std::time::Duration::from_millis(200);
        match run_limited(&dir, "(sleep 1; touch late) & wait", limit).unwrap() {
            Limited::TimedOut { kill_fault } => assert_eq!(kill_fault, None),
            Limited::Finished(..) => panic!("the command ended within 200ms"),
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(
            !dir.join("late").exists(),
            "the background child ran on after the group was killed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_stopped_at_its_limit_reports_no_fault_when_the_kill_works() {
        let dir = std::env::temp_dir();
        let limit = std::time::Duration::from_millis(200);
        match run_limited(&dir, "sleep 30", limit).unwrap() {
            Limited::TimedOut { kill_fault } => assert_eq!(kill_fault, None),
            Limited::Finished(..) => panic!("sleep 30 ended within 200ms"),
        }
        match run_limited(&dir, "echo hi", std::time::Duration::from_secs(10)).unwrap() {
            Limited::Finished(passed, out) => {
                assert!(passed);
                assert_eq!(out, b"hi\n");
            }
            Limited::TimedOut { .. } => panic!("echo ran past 10s"),
        }
    }

    #[test]
    fn the_agent_trailer_names_the_author() {
        let msg = "feat: x\n\nItem: I-1\nAgent: heron\n";
        assert!(require_agent_trailer(msg, "heron", "abc").is_ok());
        assert!(require_agent_trailer(msg, "wren", "abc").is_err());
        assert!(require_agent_trailer("feat: x\n\nItem: I-1\n", "heron", "abc").is_err());
        assert!(require_agent_trailer("Agent: heron\nAgent: wren\n", "heron", "abc").is_err());
    }

    #[test]
    fn trailers_are_read_from_message_lines() {
        let msg = "feat(core): the fold\n\nBody text.\n\nItem: I-7\nCo-Authored-By: someone\n";
        assert_eq!(item_trailers(msg), vec![Ok(ItemId(7))]);
        assert!(require_trailer(msg, ItemId(7), "abcdef123456").is_ok());
        assert!(require_trailer(msg, ItemId(8), "abcdef123456").is_err());
        assert!(require_trailer("no trailer\n", ItemId(7), "abc").is_err());
        assert!(require_trailer("Item: I-1\nItem: I-2\n", ItemId(1), "abc").is_err());
        assert!(require_trailer("Item: seven\n", ItemId(7), "abc").is_err());
    }

    #[test]
    fn a_check_records_its_output_as_evidence() {
        let dir = std::env::temp_dir().join(format!("ris-checks-test-{}", std::process::id()));
        // Process ids are reused; start from an empty directory.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = EvidenceStore::open(dir.join("evidence")).unwrap();
        let ok = run_check(
            &dir,
            "ok",
            &CheckConfig {
                command: "echo fine".into(),
            },
            &store,
        )
        .unwrap();
        assert!(ok.passed);
        let blob = store.get(ok.evidence).unwrap().unwrap();
        let text = String::from_utf8(blob).unwrap();
        assert!(text.starts_with("check: ok\ncommand: echo fine\nexit: 0\n"));
        assert!(text.contains("fine"));
        let bad = run_check(
            &dir,
            "bad",
            &CheckConfig {
                command: "echo boom >&2; exit 3".into(),
            },
            &store,
        )
        .unwrap();
        assert!(!bad.passed);
        let text = String::from_utf8(store.get(bad.evidence).unwrap().unwrap()).unwrap();
        assert!(text.contains("exit: 3"));
        assert!(text.contains("--- stderr ---\nboom"));
    }
}
