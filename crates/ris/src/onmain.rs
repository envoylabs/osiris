//! A phase's work on the main line, where a merge made elsewhere put
//! it.
//!
//! With the closing merge left to the git server (`merge_on_close =
//! false`), the phase's branch is pushed, merged there by whatever the
//! organisation uses, and the main line pulled. `ris phase close` then
//! looks here for the phase's work on the local main line as it stands,
//! and closes the phase only when it is there. Nothing is fetched or
//! pushed: only the local repository is read.
//!
//! A server merges a branch in one of three ways, and each is found:
//!
//! - a merge commit, or a fast-forward: the branch's tip is an ancestor
//!   of the main line;
//! - a rebase: every commit of the branch has a commit on the main line
//!   that makes the same change;
//! - a squash: one commit on the main line makes the whole change the
//!   branch makes since it left the main line.
//!
//! "The same change" is git's patch id, which is taken over a diff with
//! its line numbers and whitespace left out. A merge the server had to
//! resolve by hand makes a different change and is not found: the
//! person names its commit ([`named`]).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// How the phase's work reached the main line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// A merge commit brought the branch in.
    Merge,
    /// The main line was moved forward to the branch's tip.
    FastForward,
    /// The branch's commits were made again on the main line.
    Rebase,
    /// One commit carries the branch's whole change.
    Squash,
    /// The person named the commit.
    Named,
}

/// The phase's work on the main line: the commits there where it starts
/// and ends, in full, and how it got there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnMain {
    pub first: String,
    pub last: String,
    pub how: How,
}

impl OnMain {
    /// In words, after "its work is on the main line".
    pub fn said(&self) -> String {
        let (first, last) = (short(&self.first), short(&self.last));
        match self.how {
            How::Merge => format!("by the merge {last}"),
            How::FastForward => format!("fast-forwarded, from {first} to {last}"),
            How::Rebase => format!("rebased, from {first} to {last}"),
            How::Squash => format!("squashed into {last}"),
            How::Named => format!("at {last}, as named"),
        }
    }
}

/// What looking for a branch's work on the main line found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The branch holds no work the main line lacks, and none of the
    /// phase's merged commits: there is nothing to look for.
    NoWork,
    /// The work is there.
    At(OnMain),
    /// The work is not there, or not in a form this recognises.
    Missing,
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn line(dir: &Path, args: &[&str]) -> Result<String, String> {
    git(dir, args).map(|b| String::from_utf8_lossy(&b).trim().to_owned())
}

fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(line(dir, args)?
        .lines()
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect())
}

fn commit(dir: &Path, name: &str) -> Result<String, String> {
    line(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )
}

fn is_ancestor(dir: &Path, older: &str, newer: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", older, newer]).is_ok()
}

/// The patch id of the change from `from` to `to`: the same for two
/// diffs that make the same change at different places in their files.
/// `None` when there is no change, or it cannot be taken.
fn patch_id(dir: &Path, from: &str, to: &str) -> Option<String> {
    let diff = git(dir, &["diff-tree", "-p", "--no-ext-diff", from, to]).ok()?;
    if diff.is_empty() {
        return None;
    }
    let mut child = Command::new("git")
        .args(["patch-id", "--stable"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // It prints one line, once its input ends, so writing all of the
    // diff first cannot fill a pipe it is not reading.
    child.stdin.take()?.write_all(&diff).ok()?;
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

/// Look for the work of `branch` on `main`, both as they stand in the
/// repository at `dir`. `merged` is the commits of the phase's merged
/// items, oldest first: where the main line was moved forward to the
/// branch's tip, nothing in git says where the phase's work starts, and
/// the first of them on the branch does.
pub fn find(dir: &Path, main: &str, branch: &str, merged: &[String]) -> Result<Found, String> {
    let tip = commit(dir, &format!("refs/heads/{branch}"))
        .map_err(|_| format!("the branch {branch} is not in this repository"))?;
    let head = commit(dir, &format!("refs/heads/{main}"))
        .map_err(|_| format!("the main line {main} is not in this repository"))?;
    if is_ancestor(dir, &tip, &head) {
        return Ok(reachable(dir, &tip, &head, merged));
    }
    let Ok(fork) = line(dir, &["merge-base", &head, &tip]) else {
        return Ok(Found::Missing);
    };
    if let Some(found) = rebased(dir, &head, &tip)? {
        return Ok(Found::At(found));
    }
    Ok(squashed(dir, &fork, &head, &tip)?.map_or(Found::Missing, Found::At))
}

/// The branch's tip is an ancestor of the main line: a merge commit
/// brought it in, or the main line was moved forward to it.
fn reachable(dir: &Path, tip: &str, head: &str, merged: &[String]) -> Found {
    // The oldest commit on the main line's own history (its first
    // parents) that holds the tip: the merge that brought it in, or the
    // tip itself when the main line runs through it.
    let chain = lines(dir, &["rev-list", "--first-parent", head]).unwrap_or_default();
    let mut holder = head.to_owned();
    for c in chain {
        if !is_ancestor(dir, tip, &c) {
            break;
        }
        holder = c;
    }
    if holder != tip {
        return Found::At(OnMain {
            first: holder.clone(),
            last: holder,
            how: How::Merge,
        });
    }
    match merged.iter().find(|c| is_ancestor(dir, c, tip)) {
        Some(first) => Found::At(OnMain {
            first: commit(dir, first).unwrap_or_else(|_| first.clone()),
            last: tip.to_owned(),
            how: How::FastForward,
        }),
        None => Found::NoWork,
    }
}

/// Every commit of the branch that the main line lacks has a commit
/// there making the same change: the first and last of those.
fn rebased(dir: &Path, head: &str, tip: &str) -> Result<Option<OnMain>, String> {
    let both = format!("{head}...{tip}");
    let marked = |side: &str| {
        lines(
            dir,
            &["rev-list", side, "--cherry-mark", "--no-merges", &both],
        )
    };
    let ours = marked("--right-only")?;
    if ours.is_empty() || !ours.iter().all(|l| l.starts_with('=')) {
        return Ok(None);
    }
    // Newest first, as rev-list gives them.
    let theirs: Vec<String> = marked("--left-only")?
        .into_iter()
        .filter_map(|l| l.strip_prefix('=').map(str::to_owned))
        .collect();
    Ok(match (theirs.last(), theirs.first()) {
        (Some(first), Some(last)) => Some(OnMain {
            first: first.clone(),
            last: last.clone(),
            how: How::Rebase,
        }),
        _ => None,
    })
}

/// One commit on the main line since the branch left it makes the
/// whole change the branch makes.
fn squashed(dir: &Path, fork: &str, head: &str, tip: &str) -> Result<Option<OnMain>, String> {
    let Some(want) = patch_id(dir, fork, tip) else {
        return Ok(None);
    };
    let since = lines(
        dir,
        &[
            "rev-list",
            "--first-parent",
            "--no-merges",
            &format!("{fork}..{head}"),
        ],
    )?;
    Ok(since
        .into_iter()
        .find(|c| patch_id(dir, &format!("{c}^"), c).as_deref() == Some(want.as_str()))
        .map(|c| OnMain {
            first: c.clone(),
            last: c,
            how: How::Squash,
        }))
}

/// The commit the person names as carrying the phase's work, which
/// must be on the main line.
pub fn named(dir: &Path, main: &str, name: &str) -> Result<OnMain, String> {
    // A name that starts with a dash would be read as an option.
    let found = if name.starts_with('-') {
        Err(String::new())
    } else {
        commit(dir, name)
    };
    let Ok(at) = found else {
        return Err(format!("`{name}` names no commit in this repository"));
    };
    if !is_ancestor(dir, &at, &format!("refs/heads/{main}")) {
        return Err(format!("{} is not on the main line {main}", short(&at)));
    }
    Ok(OnMain {
        first: at.clone(),
        last: at,
        how: How::Named,
    })
}
