//! The exits: scanning what leaves the machine.
//!
//! Harness state is scanned as it is written. Commits leave too, so
//! `ris merge` scans a commit's message and the lines its diff adds, and
//! `ris scan` runs the same scan over files in the tree for anyone who
//! wants to look before committing. Hits name a file, a line and a
//! category; never the matched text. There is no allowance mechanism:
//! a test fixture that needs a secret-shaped string assembles it at
//! runtime so the literal is never in the tree.

use std::path::Path;
use std::process::Command;

use ris_core::text::{Category, Policy, scan};

use crate::project::CliError;

/// One line that failed the scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The file, relative to the repository root, or a label such as
    /// `commit message`.
    pub file: String,
    /// 1-based line number within the file, when known.
    pub line: Option<usize>,
    pub category: Category,
}

impl std::fmt::Display for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(n) => write!(f, "{}:{n}: {}", self.file, self.category.description()),
            None => write!(f, "{}: {}", self.file, self.category.description()),
        }
    }
}

/// Scan `text` line by line, labelling hits with `file`.
pub fn scan_lines(file: &str, text: &str, policy: &Policy) -> Vec<Hit> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            scan(line, policy).map(|category| Hit {
                file: file.to_owned(),
                line: Some(i + 1),
                category,
            })
        })
        .collect()
}

/// Scan the lines a unified diff adds. Removed and context lines are
/// not scanned: they are already in history or unchanged.
pub fn scan_diff(diff: &str, policy: &Policy) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut file = String::from("?");
    let mut line = 0usize;
    for raw in diff.lines() {
        if let Some(rest) = raw.strip_prefix("+++ ") {
            file = rest
                .strip_prefix("b/")
                .unwrap_or(rest)
                .split('\t')
                .next()
                .unwrap_or(rest)
                .to_owned();
            continue;
        }
        if raw.starts_with("--- ") || raw.starts_with("diff ") || raw.starts_with("index ") {
            continue;
        }
        if let Some(rest) = raw.strip_prefix("@@ ") {
            // "@@ -a,b +c,d @@": the new side starts at line c.
            line = rest
                .split(' ')
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|range| range.split(',').next())
                .and_then(|start| start.parse::<usize>().ok())
                .unwrap_or(1);
            continue;
        }
        match raw.chars().next() {
            Some('+') => {
                if let Some(category) = scan(&raw[1..], policy) {
                    hits.push(Hit {
                        file: file.clone(),
                        line: Some(line),
                        category,
                    });
                }
                line += 1;
            }
            Some('-') => {}
            Some('\\') => {} // "\ No newline at end of file"
            _ => line += 1,
        }
    }
    hits
}

/// The diff a commit introduces against its parent (or against nothing
/// for a root commit), with no colour and no context lines.
pub fn commit_diff(root: &Path, commit: &str) -> Result<String, CliError> {
    let out = Command::new("git")
        .args([
            "show",
            "--format=",
            "--no-color",
            "--unified=0",
            "--no-ext-diff",
            commit,
        ])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        return Err(CliError::Other(format!("git show {commit} failed")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Scan a commit's message and added lines.
pub fn scan_commit(
    root: &Path,
    commit: &str,
    message: &str,
    policy: &Policy,
) -> Result<Vec<Hit>, CliError> {
    let mut hits = scan_lines("commit message", message, policy);
    hits.extend(scan_diff(&commit_diff(root, commit)?, policy));
    Ok(hits)
}

/// The files `ris scan` reads: what git tracks plus what it would pick
/// up next (untracked and not ignored), narrowed to `paths` when any are
/// given. A path named explicitly is scanned even if git ignores it or
/// does not know it: naming a file is asking for it to be read. A named
/// path that does not exist is an error, so a typo is not reported
/// clean.
pub fn files_to_scan(root: &Path, paths: &[String]) -> Result<Vec<String>, CliError> {
    let mut cmd = Command::new("git");
    cmd.args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ])
    .current_dir(root);
    for p in paths {
        cmd.arg(p);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(CliError::Other("git ls-files failed".into()));
    }
    let mut files: std::collections::BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    for p in paths {
        let path = root.join(p);
        if !path.exists() {
            return Err(CliError::Other(format!(
                "{p}: no such file or directory\n  next: name a path relative to the repository root"
            )));
        }
        if path.is_file() {
            files.insert(p.strip_prefix("./").unwrap_or(p).to_owned());
        }
    }
    Ok(files.into_iter().collect())
}

/// Scan `files` under `root`. Files that are not UTF-8 are skipped: the scan
/// is for text, and a binary is not where a secret is typed.
pub fn scan_files(root: &Path, files: &[String], policy: &Policy) -> Result<Vec<Hit>, CliError> {
    let mut hits = Vec::new();
    for file in files {
        let bytes = match std::fs::read(root.join(file)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if let Ok(text) = std::str::from_utf8(&bytes) {
            hits.extend(scan_lines(file, text, policy));
        }
    }
    Ok(hits)
}

#[cfg(test)]
#[path = "tests/exits.rs"]
mod tests;
