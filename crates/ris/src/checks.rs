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
#[path = "tests/checks.rs"]
mod tests;
