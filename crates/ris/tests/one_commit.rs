//! At width one, as from a worktree, an item merges as exactly one
//! commit: counted from the last merge recorded in the phase (whichever
//! phase the merged item was added in), or from where
//! the phase branch left the main line before the first.

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
    assert!(out.status.success(), "git {}", args.join(" "));
}

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(dir: &Path, file: &str, item: &str) {
    fs::write(dir.join(file), format!("{file}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {file}"),
            "-m",
            &format!("Item: {item}\nAgent: heron"),
        ],
    );
}

fn setup() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ris-one-commit-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, None, &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        dir.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    // A commit on the main line before the phase opens is not counted.
    commit(&dir, "before.txt", "none");
    ok(&dir, None, &["phase", "open", "one commit each"]);
    ok(&dir, None, &["item", "add", "first", "--check", "ok"]);
    ok(&dir, None, &["item", "add", "second", "--check", "ok"]);
    dir
}

fn refusal(dir: &Path, item: &str) -> String {
    let out = ris(dir, None, &["merge", item]);
    assert!(!out.status.success(), "merge {item} should be refused");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_item_is_one_commit_from_the_phase_start_and_from_each_merge() {
    let dir = setup();
    let heron = Some("heron");

    ok(&dir, heron, &["claim", "I-1"]);
    ok(&dir, heron, &["done", "I1"]);
    // Evidence is shown by id, with or without the hyphen, in full or
    // by a prefix of at least eight characters.
    let shown = ris(&dir, None, &["--json", "show", "I1"]);
    let v: serde_json::Value =
        serde_json::from_slice::<serde_json::Value>(&shown.stdout).unwrap()["data"].clone();
    let e = v["evidence"][0].as_str().unwrap().to_owned();
    let full = String::from_utf8(ris(&dir, None, &["show", &e]).stdout).unwrap();
    assert!(full.contains("command: true"), "{full}");
    let short = format!("e{}", e[2..10].to_ascii_uppercase());
    assert_eq!(
        String::from_utf8(ris(&dir, None, &["show", &short]).stdout).unwrap(),
        full
    );
    assert!(
        !ris(&dir, None, &["show", "E-0000"]).status.success(),
        "too short to be an id"
    );
    assert!(refusal(&dir, "I-1").contains("nothing to merge"));
    commit(&dir, "a.txt", "I-1");
    commit(&dir, "b.txt", "I-1");
    let err = refusal(&dir, "I-1");
    assert!(
        err.contains("2 commits since where the phase left main"),
        "{err}"
    );
    git(&dir, &["reset", "-q", "--soft", "HEAD~2"]);
    commit(&dir, "a.txt", "I-1");
    ok(&dir, None, &["merge", "I-1"]);

    ok(&dir, heron, &["claim", "I-2"]);
    ok(&dir, heron, &["done", "I-2"]);
    commit(&dir, "c.txt", "I-2");
    commit(&dir, "d.txt", "I-2");
    let err = refusal(&dir, "I-2");
    assert!(err.contains("2 commits since the last merge"), "{err}");
    git(&dir, &["reset", "-q", "--soft", "HEAD~2"]);
    commit(&dir, "c.txt", "I-2");
    ok(&dir, None, &["merge", "I-2"]);
}

/// An item carried over from an earlier phase and merged in this one
/// counts as this phase's merge: the next item is one commit from it,
/// not from where the phase left the main line.
#[test]
fn a_carried_over_items_merge_counts_for_the_next_item() {
    let dir = setup();
    let heron = Some("heron");
    ok(&dir, heron, &["claim", "I-1"]);
    ok(&dir, heron, &["done", "I-1"]);
    commit(&dir, "a.txt", "I-1");
    ok(&dir, None, &["merge", "I-1"]);
    // I-2 is left ready and carries over.
    ok(&dir, None, &["phase", "close"]);
    ok(&dir, None, &["phase", "open", "the next phase"]);
    ok(&dir, None, &["item", "add", "third", "--check", "ok"]);

    ok(&dir, heron, &["claim", "I-2"]);
    ok(&dir, heron, &["done", "I-2"]);
    commit(&dir, "b.txt", "I-2");
    ok(&dir, None, &["merge", "I-2"]);

    ok(&dir, heron, &["claim", "I-3"]);
    ok(&dir, heron, &["done", "I-3"]);
    commit(&dir, "c.txt", "I-3");
    ok(&dir, None, &["merge", "I-3"]);
}
