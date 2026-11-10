//! `ris status`: where each seat is. Its role, its worktree and branch,
//! the item it holds, how far its branch is ahead of and behind the
//! phase branch, and whether it has uncommitted changes. Read from the
//! configuration, the log and git; nothing is recorded.
//!
//! A writing seat's worktree is the one `ris worktree` made: on a branch
//! named after the seat. An auditor reads the main checkout, and a lead
//! with no worktree of its own works there at width one.

use std::path::{Path, PathBuf};
use std::process::Command;

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
}

/// `(path, branch)` for every worktree of the repository at `root`.
fn worktrees(root: &Path) -> Vec<(PathBuf, String)> {
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

pub fn seats(p: &Project, l: &Loaded) -> Vec<Seat> {
    let trees = worktrees(&p.root);
    let phase_branch = git(&p.root, &["rev-parse", "--abbrev-ref", "HEAD"]);
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
            // Its own worktree if it has one. Otherwise an auditor, and
            // a lead working at width one, use the main checkout; a
            // builder without one has nowhere to work yet.
            let (worktree, branch) = match trees.iter().find(|(_, b)| b == name) {
                Some((path, b)) if cfg.role != Role::Auditor => {
                    (Some(path.clone()), Some(b.clone()))
                }
                _ if cfg.role != Role::Builder => (Some(p.root.clone()), phase_branch.clone()),
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
            }
        })
        .collect()
}

fn role_word(r: Role) -> &'static str {
    match r {
        Role::Lead => "lead",
        Role::Builder => "builder",
        Role::Auditor => "auditor",
    }
}

/// Paths are shown relative to the store root's parent, where `ris
/// worktree` puts them, so no absolute path reaches the screen.
fn shown(p: &Project, path: &Path) -> String {
    if path == p.root {
        return "main checkout".into();
    }
    let base = p.root.parent().unwrap_or(&p.root);
    match path.strip_prefix(base) {
        Ok(rel) => format!("../{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn render(p: &Project, l: &Loaded) -> Output {
    let seats = seats(p, l);
    let mut text = String::new();
    for s in &seats {
        let place = match &s.worktree {
            Some(w) => shown(p, w),
            None => format!("no worktree (ris worktree {})", s.seat),
        };
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
        if s.paused {
            line.push_str("  paused");
        }
        text.push_str(&line);
        text.push('\n');
    }
    if seats.is_empty() {
        text.push_str("no seats\n  next: ris seat add <name> --role lead\n");
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
            })
        })
        .collect();
    Output::new(text, json!({ "seats": rows }))
}
