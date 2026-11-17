//! `ris phase review`: the person's review step before a close. It
//! prints the phase's state, walks the open decisions, shows the change
//! set ranked by risk and records the review at the tip of the phase's
//! work. `ris phase close` refuses a tip that has not been reviewed,
//! unless told to close unreviewed, and the summary's ready line points
//! at the review until it stands.

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn command(dir: &Path, agent: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ris"));
    c.args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    c
}

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    command(dir, agent, args).output().unwrap()
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
    let out = ok(dir, "person", &a);
    common::validate_output(args, &out);
    serde_json::from_str::<Value>(&out).unwrap()["data"].clone()
}

/// The person's command with `input` on its standard input, as lines.
fn with_input(dir: &Path, args: &[&str], input: &str) -> Output {
    let mut child = command(dir, "person", args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn refused_close(dir: &Path) -> String {
    let out = ris(dir, "person", &["phase", "close"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    String::from_utf8(out.stderr).unwrap()
}

fn first_line(dir: &Path) -> String {
    ok(dir, "person", &["summary"])
        .lines()
        .next()
        .unwrap()
        .to_owned()
}

/// A repository on `main` with a project in it, and P-1 open with its
/// branch checked out here, so the person commits and merges its items.
fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-review-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn base() {}\n").unwrap();
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
        "denylist = [\"example client\"]\ndocs_item = false\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    ok(&dir, "person", &["phase", "open", "review it"]);
    git(&dir, &["switch", "-q", "phase/P-1"]);
    dir
}

/// Claim, finish and merge `item`, its commit adding a source file.
fn work(dir: &Path, item: &str) {
    ok(dir, "heron", &["claim", item]);
    ok(dir, "heron", &["done", item]);
    let file = format!("src/{}.rs", item.to_lowercase().replace('-', "_"));
    fs::write(dir.join(&file), "pub fn f() {}\n").unwrap();
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {item}"),
            "-m",
            &format!("Item: {item}\nAgent: heron"),
        ],
    );
    ok(dir, "person", &["merge", item]);
}

#[test]
fn the_close_waits_for_a_review_at_the_tip_of_the_phases_work() {
    let dir = setup("tip");
    ok(&dir, "person", &["item", "add", "first", "--check", "ok"]);
    ok(&dir, "person", &["item", "add", "second", "--check", "ok"]);
    work(&dir, "I-1");

    // Never reviewed: the close says so, and names the review and the
    // way to close without one.
    let why = refused_close(&dir);
    assert!(why.contains("P-1 has not been reviewed"), "{why}");
    assert!(why.contains("`ris phase review`"), "{why}");
    assert!(why.contains("`ris phase close --unreviewed`"), "{why}");

    // The review is the person's.
    let out = ris(&dir, "heron", &["phase", "review"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("only the person reviews the phase"),
        "{out:?}"
    );

    // While the phase is open, on what has merged so far: the state as
    // the summary gives it first, then the change set ranked, then the
    // review recorded at the tip.
    let summary = ok(&dir, "person", &["summary"]);
    let text = ok(&dir, "person", &["phase", "review"]);
    assert!(text.starts_with(summary.trim_end()), "{text}");
    let tip = git(&dir, &["rev-parse", "phase/P-1"]);
    let list = text.find("src/i_1.rs (added)").expect("the change set");
    let recorded = text
        .find(&format!("reviewed P-1 at {}", &tip[..12]))
        .expect("the review");
    assert!(list < recorded, "{text}");
    assert!(
        text.contains("`ris map` draws the same files as a graph beside this list"),
        "{text}"
    );

    // The merge of I-2 moves the phase on: the review no longer stands,
    // and the summary points at the review, not the close.
    work(&dir, "I-2");
    let line = first_line(&dir);
    assert_eq!(
        line,
        "all 2 items merged: ready to review with `ris phase review`"
    );
    assert_eq!(data(&dir, &["summary"])["phase_state"]["reviewed"], false);
    assert!(refused_close(&dir).contains("P-1 has not been reviewed"));

    // Reviewed again, in JSON: the tip and the change set are the data.
    let review = data(&dir, &["phase", "review"]);
    let tip = git(&dir, &["rev-parse", "phase/P-1"]);
    assert_eq!(review["phase"], "P-1");
    assert_eq!(review["tip"], json!(tip));
    assert_eq!(review["map"]["tip"], json!(tip));
    assert_eq!(review["open"], json!([]));
    let line = first_line(&dir);
    assert_eq!(
        line,
        "all 2 items merged and reviewed: ready to close with `ris phase close`"
    );
    assert_eq!(data(&dir, &["summary"])["phase_state"]["reviewed"], true);

    // The branch moved by a commit that is no item's: the review was of
    // another tip, and the close says so.
    fs::write(
        dir.join("src/lib.rs"),
        "pub fn base() {}\npub fn more() {}\n",
    )
    .unwrap();
    git(&dir, &["commit", "-q", "-am", "chore: by hand"]);
    // The summary reads it as the close does.
    assert_eq!(
        first_line(&dir),
        "all 2 items merged: ready to review with `ris phase review`"
    );
    assert_eq!(data(&dir, &["summary"])["phase_state"]["reviewed"], false);
    let why = refused_close(&dir);
    assert!(
        why.contains(&format!(
            "P-1 was reviewed at {}, and its work has moved since",
            &tip[..12]
        )),
        "{why}"
    );

    // Reviewed at the tip, it closes, and the close says how the
    // closed phase's change set is read.
    ok(&dir, "person", &["phase", "review"]);
    git(&dir, &["switch", "-q", "main"]);
    let closed = ok(&dir, "person", &["phase", "close"]);
    assert!(
        closed.contains(
            "P-1's change set stays readable: `ris map P-1`, or `ris map P-1 --html <file>`"
        ),
        "{closed}"
    );
    let map = data(&dir, &["map", "P-1"]);
    assert_eq!(map["phase"], "P-1");
    assert!(
        map["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["path"] == "src/i_2.rs"),
        "{map}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_review_walks_the_open_decisions_and_close_may_skip_it() {
    let dir = setup("walk");
    ok(&dir, "person", &["item", "add", "first", "--check", "ok"]);
    ok(&dir, "person", &["item", "add", "second", "--check", "ok"]);
    let raise = |item: &str, question: &str| {
        ok(&dir, "heron", &["claim", item]);
        ok(
            &dir,
            "heron",
            &[
                "decision",
                "raise",
                "--substantive",
                "--about",
                item,
                "--question",
                question,
                "--option",
                "a",
                "--option",
                "b",
            ],
        )
    };
    raise("I-1", "which way?");

    // In JSON nothing is walked: the open decisions are listed.
    let review = data(&dir, &["phase", "review"]);
    assert_eq!(review["open"], json!(["D-1"]));
    assert_eq!(review["taken"], json!([]));

    // At the terminal the walk is `ris decide`'s: a key picks, a line
    // gives the rationale.
    raise("I-2", "and then?");
    let out = with_input(&dir, &["phase", "review"], "1\nthe second\ns\n");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("which way?"), "{text}");
    assert!(text.contains("D-1 taken: option 1"), "{text}");
    assert!(text.contains("still open: D-2"), "{text}");
    assert!(
        text.contains("1 substantive decision(s) still open: `ris decide` takes them"),
        "{text}"
    );
    assert_eq!(
        data(&dir, &["decide"])["queue"].as_array().unwrap().len(),
        1
    );

    // The close may go without a review when the person says so.
    git(
        &dir,
        &["commit", "-q", "--allow-empty", "-m", "chore: move on"],
    );
    assert!(refused_close(&dir).contains("its work has moved since"));
    git(&dir, &["switch", "-q", "main"]);
    let closed = ok(&dir, "person", &["phase", "close", "--unreviewed"]);
    assert!(closed.contains("P-1 closed"), "{closed}");

    let _ = fs::remove_dir_all(&dir);
}
