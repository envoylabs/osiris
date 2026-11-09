//! Several builders, each in its own git worktree, sharing one store.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {} failed: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A repository with a phase open, `checks` as its checks file, and
/// one worktree per agent on its own branch, outside the main tree.
fn setup(name: &str, checks: &str, agents: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let base = std::env::temp_dir().join(format!("ris-width-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main = base.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "phase"]);
    fs::write(main.join("README"), "base\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    ok(&main, None, &["init"]);
    fs::write(main.join(".osiris/checks.toml"), checks).unwrap();
    ok(&main, None, &["phase", "open", "width test"]);
    let trees = agents
        .iter()
        .map(|a| {
            let wt = base.join(a);
            git(
                &main,
                &["worktree", "add", "-q", "-b", a, wt.to_str().unwrap()],
            );
            wt
        })
        .collect();
    (main, trees)
}

#[test]
fn a_worktree_finds_the_shared_store_and_checks_run_in_it() {
    let (main, trees) = setup(
        "store",
        "[checks.marker]\ncommand = \"test -f only-here.txt\"\n",
        &["heron"],
    );
    let wt = &trees[0];
    ok(
        &main,
        None,
        &["item", "add", "a thing", "--check", "marker"],
    );
    // The worktree is outside the main tree, yet sees the same store.
    let ready = ok(wt, Some("heron"), &["ready"]);
    assert!(ready.contains("I-1"), "{ready}");
    ok(wt, Some("heron"), &["claim", "I-1"]);
    // The marker exists only in the worktree, so the check passes only
    // if it runs there.
    fs::write(wt.join("only-here.txt"), "x\n").unwrap();
    assert!(!main.join("only-here.txt").exists());
    ok(wt, Some("heron"), &["done", "I-1"]);
    let report = ok(&main, None, &["report"]);
    assert!(report.contains("I-1   done"), "{report}");
}
