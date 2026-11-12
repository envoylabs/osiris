//! `ris status`: where each seat is. Its role, its worktree and branch,
//! the item it holds, how far its branch is ahead of and behind the
//! phase branch, and whether it has uncommitted changes. Read from the
//! configuration, the log and git; nothing is recorded. For a running
//! seat, also what it last did and how long ago, from its run's output
//! file, and whether it has been quiet long enough to be stuck.
//!
//! A writing seat's worktree is the one `ris worktree` made: on a branch
//! named after the seat. An auditor reads the main checkout, and a lead
//! with no worktree of its own works there at width one.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::json;

use ris_core::ids::AgentName;
use ris_core::primitives::ItemState;

use crate::config::Role;
use crate::project::{Loaded, Project};
use crate::render::Output;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// One seat's position.
#[derive(Debug, Serialize)]
pub struct Seat {
    pub seat: String,
    pub role: Role,
    /// The worktree it works in; `None` when it has none yet.
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    /// The item it holds, if any.
    pub claim: Option<String>,
    /// Commits on its branch not on the phase branch, and the reverse.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// Files with uncommitted changes in its worktree.
    pub uncommitted: Option<u32>,
    /// Paused by the person: takes nothing new until resumed.
    pub paused: bool,
    /// Started by the runner and not yet seen to stop.
    pub running: bool,
    /// For a running seat: what its latest run last did, and how long
    /// ago its output last changed.
    pub activity: Option<Activity>,
    /// Faults in its setup it reported (`ris fault`) since its latest
    /// run started, oldest first: what the person fixes in the
    /// configuration before the next `ris up`.
    pub faults: Vec<String>,
}

/// What a running seat last did, read from its run's output file.
#[derive(Debug, Serialize)]
pub struct Activity {
    /// The tool it last ran or what it last said, on one line; `None`
    /// before it has said anything.
    pub last: Option<String>,
    /// Seconds since its output last changed.
    pub quiet_seconds: u64,
    /// Quiet for longer than `[runner] quiet_after_seconds`.
    pub stuck: bool,
}

/// How long a seat's last activity line may be on screen.
const ACTIVITY_WIDTH: usize = 60;

/// A running seat's activity, from its latest run file: the file's last
/// modification is when its output last changed. `None` when it has no
/// run file or the file cannot be read.
pub fn activity(runs: &Path, seat: &str, quiet_after: u64, now: SystemTime) -> Option<Activity> {
    let path = crate::runner::latest_run(runs, seat)?;
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let quiet = now.duration_since(modified).unwrap_or(Duration::ZERO);
    Some(Activity {
        last: crate::runner::last_activity(&text, ACTIVITY_WIDTH),
        quiet_seconds: quiet.as_secs(),
        stuck: quiet > Duration::from_secs(quiet_after),
    })
}

/// A span of seconds as people say it: `45s`, `12m`, `3h`, `2d`.
pub fn ago(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// `(path, branch)` for every worktree of the repository at `root`.
pub fn worktrees(root: &Path) -> Vec<(PathBuf, String)> {
    let Some(list) = git(root, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut path = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/")
            && let Some(p) = path.take()
        {
            out.push((p, b.to_owned()));
        }
    }
    out
}

/// The open phase's branch, when phase branches are on and it exists.
/// Merges land there. It is checked out in no worktree, and a merge
/// moves it by a compare-and-swap ref update, which needs only the git
/// directory; a project whose main checkout still has it checked out
/// is moved there by a fast-forward instead.
pub fn phase_branch(p: &Project, l: &Loaded) -> Option<String> {
    if !p.config.phase_branches {
        return None;
    }
    let name = format!("phase/{}", l.state.open_phase()?.id);
    git(
        &p.root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )?;
    Some(name)
}

/// The worktree that has `branch` checked out, if any.
pub fn checked_out(root: &Path, branch: &str) -> Option<PathBuf> {
    worktrees(root)
        .into_iter()
        .find(|(_, b)| b == branch)
        .map(|(p, _)| p)
}

pub fn seats(p: &Project, l: &Loaded) -> Vec<Seat> {
    let trees = worktrees(&p.root);
    let here = git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let phase_branch = phase_branch(p, l).or_else(|| here.clone());
    let runs = p.root.join(crate::project::DIR).join("runs");
    let now = SystemTime::now();
    p.config
        .agents
        .iter()
        .map(|(name, cfg)| {
            let claim = l
                .state
                .items
                .values()
                .find(|i| matches!(&i.state, ItemState::Claimed { agent } if agent.0 == *name))
                .map(|i| i.id.to_string());
            // Its own worktree if it writes and has one. Otherwise an
            // auditor, an advisor, and a lead working at width one, use
            // the main checkout; a builder without one has nowhere to
            // work yet.
            let (worktree, branch) = match trees.iter().find(|(_, b)| b == name) {
                Some((path, b)) if cfg.role.writes() => (Some(path.clone()), Some(b.clone())),
                _ if cfg.role != Role::Builder => (Some(p.root.clone()), here.clone()),
                _ => (None, None),
            };
            let (ahead, behind) = match (&branch, &phase_branch) {
                (Some(b), Some(ph)) if b != ph => git(
                    &p.root,
                    &[
                        "rev-list",
                        "--left-right",
                        "--count",
                        &format!("{b}...{ph}"),
                    ],
                )
                .and_then(|s| {
                    let mut it = s.split_whitespace().map(|n| n.parse::<u32>().ok());
                    Some((it.next()??, it.next()??))
                })
                .map_or((None, None), |(a, b)| (Some(a), Some(b))),
                (Some(_), Some(_)) => (Some(0), Some(0)),
                _ => (None, None),
            };
            let running = l.state.running.contains_key(&AgentName(name.clone()));
            let uncommitted = worktree
                .as_deref()
                .and_then(|w| git(w, &["status", "--porcelain"]).map(|s| s.lines().count() as u32));
            Seat {
                seat: name.clone(),
                role: cfg.role,
                worktree,
                branch,
                claim,
                ahead,
                behind,
                uncommitted,
                paused: l.state.paused.contains(&AgentName(name.clone())),
                running,
                activity: running
                    .then(|| activity(&runs, name, p.config.runner.quiet_after_seconds, now))
                    .flatten(),
                faults: l
                    .state
                    .faults
                    .get(&AgentName(name.clone()))
                    .into_iter()
                    .flatten()
                    .map(|at| l.text(*at, "what").to_owned())
                    .collect(),
            }
        })
        .collect()
}

fn role_word(r: Role) -> &'static str {
    r.as_str()
}

/// Paths are shown relative to the main checkout, wherever `worktrees`
/// puts them, so no absolute path reaches the screen.
fn shown(p: &Project, path: &Path) -> String {
    if path == p.root {
        return "main checkout".into();
    }
    crate::config::relative(&p.root, path).display().to_string()
}

/// A seat as text, `place` being its worktree as shown: everything
/// about the seat on its first line, and for a running seat its last
/// activity on a second line beneath it.
fn seat_lines(s: &Seat, place: &str) -> String {
    let mut line = format!("{:<10} {:<8} {}", s.seat, role_word(s.role), place);
    if let Some(b) = &s.branch {
        line.push_str(&format!("  [{b}]"));
    }
    if let (Some(a), Some(b)) = (s.ahead, s.behind)
        && (a, b) != (0, 0)
    {
        line.push_str(&format!("  +{a} -{b}"));
    }
    if let Some(n) = s.uncommitted.filter(|n| *n > 0) {
        line.push_str(&format!("  {n} uncommitted"));
    }
    line.push_str(&match &s.claim {
        Some(c) => format!("  holds {c}"),
        None => "  idle".into(),
    });
    if s.running {
        line.push_str("  running");
    }
    if s.paused {
        line.push_str("  paused");
    }
    if let Some(a) = &s.activity {
        if a.stuck {
            line.push_str(&format!("  possibly stuck: quiet {}", ago(a.quiet_seconds)));
        }
        line.push_str(&format!(
            "\n{:<10} last {} ago: {}",
            "",
            ago(a.quiet_seconds),
            a.last.as_deref().unwrap_or("(nothing said yet)")
        ));
    }
    for f in &s.faults {
        line.push_str(&format!("\n{:<10} setup fault: {f}", ""));
    }
    line
}

pub fn render(p: &Project, l: &Loaded) -> Output {
    let seats = seats(p, l);
    let mut text = String::new();
    for s in &seats {
        let place = match &s.worktree {
            Some(w) => shown(p, w),
            None => format!("no worktree (ris worktree {})", s.seat),
        };
        text.push_str(&seat_lines(s, &place));
        text.push('\n');
    }
    if seats.is_empty() {
        text.push_str("no seats\n  next: ris seat add <name> --role lead\n");
    }
    if seats.iter().any(|s| !s.faults.is_empty()) {
        text.push_str(
            "a setup fault is something a seat's sandbox or permissions do not allow\n  \
             next: allow it in [runner] in .osiris/config.toml, then `ris up <seat>`\n",
        );
    }
    text.push_str("--\n+n -n: commits ahead of and behind the phase branch");
    let rows: Vec<_> = seats
        .iter()
        .map(|s| {
            json!({
                "seat": s.seat,
                "role": s.role,
                "worktree": s.worktree.as_deref().map(|w| shown(p, w)),
                "branch": s.branch,
                "claim": s.claim,
                "ahead": s.ahead,
                "behind": s.behind,
                "uncommitted": s.uncommitted,
                "paused": s.paused,
                "running": s.running,
                "activity": s.activity,
                "faults": s.faults,
            })
        })
        .collect();
    Output::new(text, json!({ "seats": rows }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A running seat that is also paused says so on its own line, not
    /// at the end of the activity line beneath it.
    #[test]
    fn a_paused_running_seat_says_paused_on_its_own_line() {
        let s = Seat {
            seat: "heron".into(),
            role: Role::Builder,
            worktree: None,
            branch: Some("heron".into()),
            claim: Some("I-3".into()),
            ahead: Some(0),
            behind: Some(0),
            uncommitted: Some(0),
            paused: true,
            running: true,
            activity: Some(Activity {
                last: Some("→ Bash ris done I-3".into()),
                quiet_seconds: 5,
                stuck: false,
            }),
            faults: vec!["cannot write the build cache".into()],
        };
        let text = seat_lines(&s, "../heron");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(
            lines[2].trim_start(),
            "setup fault: cannot write the build cache"
        );
        assert!(
            lines[0].starts_with("heron") && lines[0].ends_with("running  paused"),
            "{text}"
        );
        assert!(
            lines[1].trim_start().starts_with("last 5s ago: ") && !lines[1].contains("paused"),
            "{text}"
        );
    }
}
