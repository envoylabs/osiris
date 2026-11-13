//! `ris summary` opens with the open phase's state in one line: how
//! many of its items are merged, in progress and with which seats, set
//! aside and not started, what waits on the person with the command
//! for each, and whether any seat is running. `--json` carries the
//! same state as `phase_state`. The counts add up to the item list
//! below the line, superseded items left out.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
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

/// The summary's first line, and its `phase_state`, which must agree.
fn state(dir: &Path) -> (String, Value) {
    let text = ok(dir, "person", &["summary"]);
    let line = text.lines().next().unwrap().to_owned();
    let out = ok(dir, "person", &["--json", "summary"]);
    common::validate_output(&["summary"], &out);
    let v = serde_json::from_str::<Value>(&out).unwrap()["data"]["phase_state"].clone();
    assert_eq!(v["line"], line, "the text and the JSON say the same");
    (line, v)
}

fn work(dir: &Path, item: &str) {
    ok(dir, "heron", &["claim", item]);
    ok(dir, "heron", &["done", item]);
    merge(dir, item);
}

fn merge(dir: &Path, item: &str) {
    fs::write(dir.join(item), format!("{item}\n")).unwrap();
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

fn setup() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-summary-state-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
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
    dir
}

#[test]
fn summary_opens_with_the_phases_state_in_one_line() {
    let dir = setup();
    const NO_SEAT: &str = "no seat is running: start them with `ris up`";

    // No phase open: no line, and no state.
    let text = ok(&dir, "person", &["summary"]);
    assert!(text.starts_with("no phase is open"), "{text}");
    let out = ok(&dir, "person", &["--json", "summary"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"]["phase_state"], Value::Null);

    // An open phase with nothing in it says so and how to add work.
    ok(&dir, "person", &["phase", "open", "one line"]);
    git(&dir, &["switch", "-q", "phase/P-1"]);
    let (line, v) = state(&dir);
    assert_eq!(line, "no items yet: add one with `ris item add \"<goal>\"`");
    assert_eq!(v["items"], 0);
    assert_eq!(v["ready_to_close"], false);

    // Nothing started, and nobody to start it.
    for goal in ["first", "second", "third", "replaced"] {
        ok(&dir, "person", &["item", "add", goal, "--check", "ok"]);
    }
    // A superseded item is not one of the phase's N.
    ok(&dir, "person", &["item", "supersede", "I-4", "--by", "I-3"]);
    let (line, v) = state(&dir);
    assert_eq!(
        line,
        format!("0 of 3 items merged; 3 not started; {NO_SEAT}")
    );
    assert_eq!(v["phase"], "P-1");
    assert_eq!(v["running"], json!([]));

    // The header and the item list follow the line, as they were.
    let text = ok(&dir, "person", &["summary"]);
    let rest = text.split_once('\n').unwrap().1;
    assert!(
        rest.starts_with("P-1 open: one line\nitems: 3 ready, 1 superseded\n\n"),
        "{text}"
    );

    // Claimed: in progress, with the seat.
    ok(&dir, "heron", &["claim", "I-1"]);
    let (line, v) = state(&dir);
    assert_eq!(
        line,
        format!("0 of 3 items merged; 1 in progress (heron on I-1); 2 not started; {NO_SEAT}")
    );
    assert_eq!(
        v["in_progress"],
        json!([{ "item": "I-1", "seat": "heron" }])
    );

    // Set aside on a substantive decision, which waits on the person.
    ok(
        &dir,
        "heron",
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-1",
            "--question",
            "which way?",
            "--option",
            "a",
            "--option",
            "b",
        ],
    );
    let (line, v) = state(&dir);
    assert_eq!(
        line,
        format!(
            "0 of 3 items merged; 1 set aside; 2 not started; \
             1 open decision(s) wait on you: `ris decide`; {NO_SEAT}"
        )
    );
    assert_eq!(v["set_aside"], json!(["I-1"]));
    assert_eq!(v["open_decisions"], json!(["D-1"]));

    // Done and not yet merged is still in progress, with its author.
    ok(&dir, "heron", &["claim", "I-2"]);
    ok(&dir, "heron", &["done", "I-2"]);
    let (line, _) = state(&dir);
    assert!(
        line.starts_with(
            "0 of 3 items merged; 1 in progress (heron on I-2); 1 set aside; 1 not started; "
        ),
        "{line}"
    );
    merge(&dir, "I-2");
    let (line, v) = state(&dir);
    assert!(
        line.starts_with("1 of 3 items merged; 1 set aside; 1 not started; "),
        "{line}"
    );
    // The four counts add up to the items.
    let n = |k: &str| v[k].as_u64().unwrap();
    let len = |k: &str| v[k].as_array().unwrap().len() as u64;
    assert_eq!(
        n("merged") + len("in_progress") + len("set_aside") + n("not_started"),
        n("items")
    );

    // The decision taken, nothing waits on the person any more.
    ok(&dir, "person", &["decide", "D-1", "--choose", "0"]);
    let (line, v) = state(&dir);
    assert_eq!(
        line,
        format!("1 of 3 items merged; 2 not started; {NO_SEAT}")
    );
    assert_eq!(v["open_decisions"], json!([]));

    // A setup fault waits on the person, with what deals with it.
    ok(
        &dir,
        "heron",
        &["seat", "fault", "cannot reach the registry"],
    );
    let (line, v) = state(&dir);
    assert!(
        line.contains(
            "a setup fault on heron waits on you: `ris status` says what, then `ris up heron`"
        ),
        "{line}"
    );
    assert_eq!(v["faults"], json!(["heron"]));

    // Everything merged: ready to close, and no call for `ris up`.
    work(&dir, "I-1");
    work(&dir, "I-3");
    let (line, v) = state(&dir);
    assert!(
        line.starts_with("all 3 items merged: ready to close with `ris phase close`"),
        "{line}"
    );
    assert!(
        !line.contains("ris up`;") && !line.contains(NO_SEAT),
        "{line}"
    );
    assert_eq!(v["ready_to_close"], true);
    assert_eq!(v["merged"], 3);

    // `--all` carries the same state; its text is the long view, as
    // it was, without the line.
    let out = ok(&dir, "person", &["--json", "summary", "--all"]);
    let all: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(all["data"]["phase_state"], v);
    let text = ok(&dir, "person", &["summary", "--all"]);
    assert!(text.starts_with("== decisions =="), "{text}");

    let _ = fs::remove_dir_all(&dir);
}
