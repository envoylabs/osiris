//! `ris phase open --branch <name>`: the phase's branch takes a name
//! of the project's own, and everything that reads the phase's branch
//! reads that name: an item's merge, the map of the open phase, the
//! closing merge and its messages, and the map of a closed phase whose
//! branch is still there. An open under a name the branch cannot take
//! is refused with nothing created.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn ok(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn data(dir: &Path, args: &[&str]) -> Value {
    let mut a = vec!["--json"];
    a.extend_from_slice(args);
    let v: Value = serde_json::from_str(&ok(dir, "person", &a)).unwrap();
    v["data"].clone()
}

/// Why `ris` refused, having failed.
fn refused(dir: &Path, args: &[&str]) -> String {
    let out = ris(dir, "person", args);
    assert!(!out.status.success(), "ris {} passed", args.join(" "));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// One source file committed as `item`'s work on the branch checked
/// out.
fn commit(dir: &Path, file: &str, item: &str) {
    fs::write(dir.join(file), "pub fn work() {}\n").unwrap();
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

/// A repository on `main` with a project in it at width one. `more` is
/// added to the top of its configuration.
fn setup(name: &str, more: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-phase-branch-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("lib.rs"), "pub fn base() {}\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, "person", &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        dir.join(".osiris/config.toml"),
        format!(
            "denylist = [\"example client\"]\ndocs_item = false\n{more}[agents.heron]\nrole = \"lead\"\n"
        ),
    )
    .unwrap();
    dir
}

fn branches(dir: &Path) -> Vec<String> {
    git(dir, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_phase_runs_from_open_to_close_on_the_branch_it_was_given() {
    let dir = setup("run", "");
    let opened = ok(
        &dir,
        "person",
        &["phase", "open", "the work", "--branch", "release/first"],
    );
    assert!(
        opened.starts_with("opened P-1 on branch release/first"),
        "{opened}"
    );
    assert_eq!(branches(&dir), ["main", "release/first"]);

    // The summary and the report name it, in text and as data.
    for view in ["summary", "report"] {
        let text = ok(&dir, "person", &[view]);
        assert!(text.contains("\nbranch: release/first\n"), "{view}: {text}");
    }
    assert_eq!(
        data(&dir, &["report"])["phase"]["branch"],
        json!("release/first")
    );
    assert_eq!(
        data(&dir, &["summary"])["report"]["phase"]["branch"],
        json!("release/first")
    );

    // An item's merge lands on it.
    git(&dir, &["switch", "-q", "release/first"]);
    ok(&dir, "person", &["item", "add", "the one", "--check", "ok"]);
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    commit(&dir, "work.rs", "I-1");
    let merged = ok(&dir, "heron", &["merge", "I-1"]);
    assert!(merged.contains("I-1 merged at"), "{merged}");
    let tip = git(&dir, &["rev-parse", "release/first"]);
    assert!(merged.contains(&tip), "{merged}");
    ok(&dir, "person", &["status"]);
    git(&dir, &["switch", "-q", "main"]);

    // The open phase's map is read from it.
    let map = data(&dir, &["map"]);
    assert_eq!(map["tip_ref"], "release/first", "{map}");
    assert_eq!(map["tip"], json!(tip));
    assert_eq!(map["base"], json!(git(&dir, &["rev-parse", "main"])));
    assert_eq!(map["totals"]["changed"], 1, "{map}");

    // The closing merge is of that branch, says so, and deletes it.
    let closed = ok(&dir, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains("closing merge: release/first into main at "),
        "{closed}"
    );
    assert!(closed.contains("; release/first deleted"), "{closed}");
    assert_eq!(branches(&dir), ["main"]);
    assert!(dir.join("work.rs").exists(), "the work is on the main line");
    assert_eq!(
        git(&dir, &["log", "-1", "--format=%s"]),
        "chore: close P-1: the work"
    );
    // Its branch gone, the closed phase's map is its closing merge's.
    let after = data(&dir, &["map", "P-1"]);
    assert_eq!(after["tip_ref"], "the closing merge of P-1 on main");
    assert_eq!(after["totals"]["changed"], 1, "{after}");
    assert_eq!(
        data(&dir, &["report"])["phase"]["branch"],
        json!("release/first"),
        "the closed phase keeps the name it ran under"
    );

    // The next phase, given no name, takes the default, which neither
    // view spells out.
    let opened = ok(&dir, "person", &["phase", "open", "the next"]);
    assert!(
        opened.starts_with("opened P-2 on branch phase/P-2"),
        "{opened}"
    );
    assert_eq!(branches(&dir), ["main", "phase/P-2"]);
    for view in ["summary", "report"] {
        let text = ok(&dir, "person", &[view]);
        assert!(!text.contains("branch:"), "{view}: {text}");
    }
    assert_eq!(data(&dir, &["report"])["phase"]["branch"], Value::Null);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_phase_whose_named_branch_is_still_there_is_mapped_from_it() {
    // The merge into the main line is left to the git server, so the
    // close leaves the branch where it is.
    let dir = setup("kept", "merge_on_close = false\n");
    ok(
        &dir,
        "person",
        &["phase", "open", "kept", "--branch", "feature/kept"],
    );
    git(&dir, &["switch", "-q", "feature/kept"]);
    ok(&dir, "person", &["item", "add", "the one", "--check", "ok"]);
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    commit(&dir, "work.rs", "I-1");
    ok(&dir, "heron", &["merge", "I-1"]);
    git(&dir, &["switch", "-q", "main"]);

    let closed = ok(&dir, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains("push feature/kept and merge it into main there"),
        "{closed}"
    );
    assert_eq!(branches(&dir), ["feature/kept", "main"]);
    let map = data(&dir, &["map", "P-1"]);
    assert_eq!(map["tip_ref"], "feature/kept", "{map}");
    assert_eq!(map["tip"], json!(git(&dir, &["rev-parse", "feature/kept"])));
    assert_eq!(map["totals"]["changed"], 1, "{map}");
    let _ = fs::remove_dir_all(&dir);
}

/// After a refused open: no phase, and no branch but those there were.
fn nothing_opened(dir: &Path, were: &[&str]) {
    assert_eq!(ok(dir, "person", &["report"]).trim(), "no phase yet");
    assert_eq!(branches(dir), were);
}

#[test]
fn a_name_the_branch_cannot_take_is_refused_with_nothing_created() {
    let dir = setup("refused", "");
    git(&dir, &["branch", "taken"]);
    let were = ["main", "taken"];
    let open = |name: &str| refused(&dir, &["phase", "open", "the work", "--branch", name]);

    // Not a name git takes.
    for name in ["two words", "ends.lock", "a..b"] {
        let err = open(name);
        assert!(
            err.contains(&format!("`{name}` is not a valid branch name")),
            "{name}: {err}"
        );
        nothing_opened(&dir, &were);
    }

    // The main line.
    let err = open("main");
    assert!(err.contains("`main` is the main line"), "{err}");
    nothing_opened(&dir, &were);

    // A branch there already is.
    let err = open("taken");
    assert!(err.contains("the branch `taken` already exists"), "{err}");
    nothing_opened(&dir, &were);
    assert_eq!(
        git(&dir, &["rev-parse", "taken"]),
        git(&dir, &["rev-parse", "main"]),
        "the branch there was is as it was"
    );

    // A name is text in the log like any other, and is scanned as it
    // goes in. This one is an address, put together here so that this
    // file holds none.
    let address = format!("someone{}example.org", '@');
    let err = open(&address);
    assert!(err.contains("branch"), "{err}");
    assert!(!err.contains(&address), "a refusal never echoes: {err}");
    nothing_opened(&dir, &were);

    // And a name the branch can take opens the phase.
    ok(
        &dir,
        "person",
        &["phase", "open", "the work", "--branch", "feature/ok"],
    );
    assert_eq!(branches(&dir), ["feature/ok", "main", "taken"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_name_is_refused_where_phases_have_no_branch() {
    let dir = setup("off", "phase_branches = false\n");
    let err = refused(
        &dir,
        &["phase", "open", "the work", "--branch", "feature/x"],
    );
    assert!(err.contains("phase branches are off"), "{err}");
    assert!(
        err.contains("next: open the phase without --branch"),
        "{err}"
    );
    nothing_opened(&dir, &["main"]);
    // Without a name it opens, with no branch, as it did.
    let opened = ok(&dir, "person", &["phase", "open", "the work"]);
    assert!(opened.starts_with("opened P-1"), "{opened}");
    assert!(!opened.contains("on branch"), "{opened}");
    assert_eq!(branches(&dir), ["main"]);
    let _ = fs::remove_dir_all(&dir);
}
