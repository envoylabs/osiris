//! The person's `ris` is the last build that passed every check (D-48).
//!
//! `ris merge` runs every check on a commit before it records it, so the
//! commit of the last `item_merged` event is the last good build. A
//! merge runs in the merging seat's sandbox, which cannot write outside
//! the seat's worktree, so it only records. The person's own `ris`,
//! which runs outside any sandbox, builds that commit and installs it:
//! on `ris install`, and on any other command of the person's once a
//! merge has moved the last good build on.
//!
//! The build runs in a detached checkout of the commit kept outside the
//! repository, so a seat's work in progress never reaches it: tracked
//! files are reset to the commit and untracked ones removed before each
//! build, while ignored files (a build cache) are kept. The binary is
//! copied beside its destination and renamed over it, so the installed
//! program is never half written. Off unless `[install]` is configured.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ris_core::event::{Event, Structure};
use ris_core::ids::ItemId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project::CliError;

/// `[install]` in `config.toml`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    /// The build command, run by `sh -c` in the checkout of the commit,
    /// for example `cargo build --release -p ris`.
    pub build: String,
    /// The built program, relative to the checkout, for example
    /// `target/release/ris`.
    pub artifact: String,
    /// Where it is installed: a path on the person's PATH. `~/` is the
    /// home directory.
    pub to: String,
    /// Where the checkout and the record of what was installed live,
    /// outside the repository and every seat's sandbox. `~/` is the home
    /// directory. Unset: `osiris/<repo>-<id>` in the user's cache
    /// directory, where `<id>` tells apart checkouts with one name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// The last good build: the commit and item of the last merge in `log`.
pub fn last_merged(log: &[Event]) -> Option<(ItemId, String)> {
    log.iter().rev().find_map(|e| match &e.structure {
        Structure::ItemMerged { item, commit } => Some((*item, commit.0.clone())),
        _ => None,
    })
}

/// `~/x` with the home directory in front; anything else as it is.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// Where an installation's files live.
pub struct Paths {
    /// The detached checkout the build runs in.
    pub src: PathBuf,
    /// The commit last installed, in full.
    pub installed: PathBuf,
    /// The commit whose build last failed, so it is not retried on every
    /// command; `ris install` retries it.
    pub failed: PathBuf,
    /// The installed program.
    pub to: PathBuf,
}

impl InstallConfig {
    pub fn paths(&self, root: &Path) -> Paths {
        let dir = match &self.dir {
            Some(d) => expand(d),
            None => {
                let cache = std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| expand("~/.cache"));
                let repo = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "repo".into());
                let id = Sha256::digest(root.as_os_str().as_encoded_bytes());
                let id: String = id[..4].iter().map(|b| format!("{b:02x}")).collect();
                cache.join("osiris").join(format!("{repo}-{id}"))
            }
        };
        Paths {
            src: dir.join("src"),
            installed: dir.join("installed"),
            failed: dir.join("failed"),
            to: expand(&self.to),
        }
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text.trim().to_owned())
    } else {
        // The last lines say why; the rest is the build's progress.
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(15)..].join("\n");
        Err(format!("{program} {} failed:\n{tail}", args.join(" ")))
    }
}

/// Build `commit` in the checkout and install the program. The commit
/// is resolved in `root`'s repository; the full hash installed is
/// returned.
pub fn install(root: &Path, cfg: &InstallConfig, commit: &str) -> Result<String, CliError> {
    let paths = cfg.paths(root);
    let full = run(
        root,
        "git",
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .map_err(CliError::Other)?;
    let attempt = || -> Result<(), String> {
        if paths.src.join(".git").exists() {
            run(
                &paths.src,
                "git",
                &["checkout", "-q", "--detach", "--force", &full],
            )?;
        } else {
            if let Some(parent) = paths.src.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // A checkout removed by hand leaves its registration behind.
            run(root, "git", &["worktree", "prune"])?;
            let src = paths.src.to_string_lossy();
            run(
                root,
                "git",
                &["worktree", "add", "-q", "--detach", "--force", &src, &full],
            )?;
        }
        // Untracked files go; ignored ones (the build cache) stay.
        run(&paths.src, "git", &["clean", "-q", "-ffd"])?;
        run(&paths.src, "sh", &["-c", &cfg.build])?;
        let artifact = paths.src.join(&cfg.artifact);
        let to = &paths.to;
        let parent = to
            .parent()
            .ok_or_else(|| "`to` has no directory".to_owned())?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let name = to
            .file_name()
            .ok_or_else(|| "`to` names no file".to_owned())?
            .to_string_lossy();
        let staged = parent.join(format!(".{name}.ris-install"));
        fs::copy(&artifact, &staged).map_err(|e| format!("copying the built program: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        fs::rename(&staged, to).map_err(|e| format!("installing: {e}"))?;
        Ok(())
    };
    match attempt() {
        Ok(()) => {
            fs::write(&paths.installed, format!("{full}\n"))?;
            let _ = fs::remove_file(&paths.failed);
            Ok(full)
        }
        Err(e) => {
            if let Some(parent) = paths.failed.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&paths.failed, format!("{full}\n"));
            Err(CliError::Other(format!(
                "could not install the build of {}: {e}\n  next: fix the build on the phase branch, then `ris install`",
                short(&full)
            )))
        }
    }
}

/// Install the last good build when it is not what is installed and
/// its build has not already failed. `None` when there is nothing to
/// do: no `[install]`, no merge yet, or already current.
pub fn catch_up(root: &Path, cfg: &InstallConfig, log: &[Event]) -> Option<String> {
    let (item, commit) = last_merged(log)?;
    let paths = cfg.paths(root);
    let full = run(
        root,
        "git",
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .ok()?;
    if read_trimmed(&paths.installed).as_deref() == Some(full.as_str())
        || read_trimmed(&paths.failed).as_deref() == Some(full.as_str())
    {
        return None;
    }
    Some(match install(root, cfg, &full) {
        Ok(_) => format!(
            "installed ris from {} ({item}, the last merge) to {}",
            short(&full),
            cfg.to
        ),
        Err(e) => format!("warning: {e}"),
    })
}

/// A commit as shown: its first twelve characters.
pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}
