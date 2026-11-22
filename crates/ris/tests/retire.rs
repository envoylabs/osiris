//! `ris item retire`: the person takes items that will not be built out
//! of the work for good, with one reason and no replacement. A retired
//! item is never offered, claimed, brought in or tagged, is not carried
//! over, is left out of `ris summary --all`, and the report counts it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
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
        "{agent}: ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The command is refused (exit 2); its stderr, for the reason.
fn refused(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{agent}: ris {} should be refused: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stderr).unwrap()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    serde_json::from_str(&ok(dir, "person", &all)).unwrap()
}

/// A fresh project in a directory of the test's own, named in its
/// canonical form, with no phase branches and no docs item.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-retire-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    ok(&dir, "person", &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace(
                "denylist = []",
                "denylist = [\"example client\"]\ndocs_item = false",
            )
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    fs::write(dir.join("note.txt"), "done\n").unwrap();
    dir
}

#[test]
fn each_refusal_holds_and_nothing_is_retired_by_a_refused_command() {
    let dir = project("refusals");
    ok(&dir, "person", &["phase", "open", "retire"]);
    ok(&dir, "person", &["item", "add", "base", "--no-checks"]);
    ok(
        &dir,
        "person",
        &[
            "item",
            "add",
            "built on base",
            "--no-checks",
            "--depends",
            "I-1",
        ],
    );
    ok(&dir, "person", &["item", "add", "held", "--no-checks"]);
    ok(&dir, "heron", &["claim", "I-3"]);

    // Only the person.
    let e = refused(&dir, "heron", &["item", "retire", "I-1", "--reason", "no"]);
    assert!(e.contains("only the person retires"), "{e}");

    // A reason that says nothing.
    let e = refused(&dir, "person", &["item", "retire", "I-2", "--reason", "  "]);
    assert!(e.contains("needs a reason"), "{e}");

    // An item a seat is working on.
    let e = refused(&dir, "person", &["item", "retire", "I-3", "--reason", "no"]);
    assert!(e.contains("I-3 is claimed"), "{e}");

    // An item that one left in the work waits on, naming the waiter.
    let e = refused(&dir, "person", &["item", "retire", "I-1", "--reason", "no"]);
    assert!(e.contains("I-2 waits on I-1"), "{e}");

    // A refusal of one item refuses the whole command: I-2 is fine on
    // its own, but the claimed I-3 beside it stops both.
    refused(
        &dir,
        "person",
        &["item", "retire", "I-2", "I-3", "--reason", "no"],
    );
    let v = json(&dir, &["show", "I-2"]);
    assert_eq!(v["data"]["state"], "ready", "{v}");

    // No such item.
    refused(
        &dir,
        "person",
        &["item", "retire", "I-99", "--reason", "no"],
    );
}

#[test]
fn several_items_retire_in_one_command_with_one_reason() {
    let dir = project("several");
    ok(&dir, "person", &["phase", "open", "retire"]);
    ok(&dir, "person", &["item", "add", "base", "--no-checks"]);
    ok(
        &dir,
        "person",
        &[
            "item",
            "add",
            "built on base",
            "--no-checks",
            "--depends",
            "I-1",
        ],
    );
    ok(&dir, "person", &["item", "add", "kept", "--no-checks"]);

    // Retired with what waits on it, in the same command.
    let out = ok(
        &dir,
        "person",
        &[
            "item",
            "retire",
            "I-1",
            "I-2",
            "I-1",
            "--reason",
            "the client dropped it",
        ],
    );
    assert_eq!(out.trim(), "retired I-1, I-2");

    for item in ["I-1", "I-2"] {
        let shown = ok(&dir, "person", &["show", item]);
        assert!(shown.contains("retired: the client dropped it"), "{shown}");
        let v = json(&dir, &["show", item]);
        assert_eq!(v["data"]["state"], "retired", "{v}");
        assert_eq!(v["data"]["retired_reason"], "the client dropped it", "{v}");
    }

    // Final: never offered, claimed, retired again, or depended on.
    let ready = ok(&dir, "person", &["ready"]);
    assert!(!ready.contains("I-1") && !ready.contains("I-2"), "{ready}");
    assert!(ready.contains("I-3"), "{ready}");
    refused(&dir, "heron", &["claim", "I-1"]);
    refused(
        &dir,
        "person",
        &["item", "retire", "I-1", "--reason", "again"],
    );
    let e = refused(
        &dir,
        "person",
        &[
            "item",
            "add",
            "needs base",
            "--no-checks",
            "--depends",
            "I-1",
        ],
    );
    assert!(e.contains("I-1 is retired"), "{e}");
}

#[test]
fn a_retired_item_is_not_brought_in_tagged_or_carried_over_and_the_report_counts_it() {
    let dir = project("counts");
    // P-1: one item done and never merged, one set aside on a decision,
    // one ready, one left in the work.
    ok(&dir, "person", &["phase", "open", "first"]);
    ok(
        &dir,
        "person",
        &["item", "add", "done never merged", "--no-checks"],
    );
    ok(&dir, "person", &["item", "add", "set aside", "--no-checks"]);
    ok(
        &dir,
        "person",
        &["item", "add", "never started", "--no-checks"],
    );
    ok(
        &dir,
        "person",
        &["item", "add", "still wanted", "--no-checks"],
    );
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1", "--evidence", "note.txt"]);
    ok(&dir, "heron", &["claim", "I-2"]);
    ok(
        &dir,
        "heron",
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-2",
            "--question",
            "which way",
            "--option",
            "a",
            "--option",
            "b",
            "--recommend",
            "0",
        ],
    );
    let v = json(&dir, &["show", "I-2"]);
    assert_eq!(v["data"]["state"], "blocked", "{v}");

    let before = json(&dir, &["report"]);
    assert_eq!(before["data"]["retired"], 0, "{before}");
    assert!(
        !ok(&dir, "person", &["report"]).contains("retired:"),
        "no line while none is retired"
    );

    ok(
        &dir,
        "person",
        &[
            "item",
            "retire",
            "I-1",
            "I-2",
            "I-3",
            "--reason",
            "not needed",
        ],
    );

    let report = ok(&dir, "person", &["report"]);
    assert!(
        report.contains("retired: 3 items, out of the work for good"),
        "{report}"
    );
    for goal in ["done never merged", "set aside", "never started"] {
        assert!(!report.contains(goal), "{goal} is listed: {report}");
    }
    let v = json(&dir, &["report"]);
    assert_eq!(v["data"]["retired"], 3, "{v}");
    assert_eq!(
        v["data"]["tally"]["carried_over"], 1,
        "only I-4 is carried over: {v}"
    );
    let ids: Vec<&str> = v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["I-4"], "{v}");

    ok(&dir, "person", &["phase", "close", "--unreviewed"]);

    // In the backlog now, but neither brought in nor tagged.
    ok(&dir, "person", &["phase", "open", "second"]);
    refused(&dir, "person", &["item", "bring-in", "I-3"]);
    refused(&dir, "person", &["item", "phase", "I-3", "P-3"]);
    refused(&dir, "person", &["item", "phase", "I-3", "none"]);

    // The long view leaves it out: not listed, not drawn, not counted as
    // left out; the still-wanted item is there.
    let all = ok(&dir, "person", &["summary", "--all"]);
    for goal in ["done never merged", "set aside", "never started"] {
        assert!(!all.contains(goal), "{goal} is in summary --all: {all}");
    }
    assert!(all.contains("still wanted"), "{all}");
    assert!(all.contains("retired: 3 items"), "{all}");
    let v = json(&dir, &["report"]);
    assert_eq!(v["data"]["tally"]["carried_over"], 1, "{v}");
}
