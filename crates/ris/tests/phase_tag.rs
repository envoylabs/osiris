//! An item's phase is a tag, so planning lives in ris between phases:
//! an item is added with no phase open, tagged for a later phase or for
//! none, in the phase it is tagged for when that phase opens, and
//! untagged again when its phase closes without it merging. Planning a
//! later phase is the person's and the advisor's.

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

/// The refusal's text; the command must exit 2.
fn refused(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{agent}: ris {}: {err}",
        args.join(" ")
    );
    err
}

fn json(dir: &Path, agent: &str, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let v: Value = serde_json::from_str(&ok(dir, agent, &all)).unwrap();
    v["data"].clone()
}

/// The phase of `item` as `ris show` gives it: a phase id, or null.
fn phase_of(dir: &Path, item: &str) -> Value {
    json(dir, "person", &["show", item])["phase"].clone()
}

/// A project with the template's seats (heron leads, wren audits) and
/// an advisor, kite; no phase branches and no docs item.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-phase-tag-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
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
    ok(
        &dir,
        "person",
        &["seat", "add", "kite", "--role", "advisor"],
    );
    dir
}

#[test]
fn planning_happens_with_no_phase_open_and_the_next_phase_opens_with_it() {
    let dir = project("plan");

    // Nothing is refused for lack of a phase: the item has none.
    let added = ok(&dir, "person", &["item", "add", "someday"]);
    assert!(added.contains("added I-1 with no phase"), "{added}");
    assert!(phase_of(&dir, "I-1").is_null());
    assert!(
        ok(&dir, "person", &["show", "I-1"]).contains("(no phase)"),
        "show says it has none"
    );

    // Tagged as it is added, by the person and by the advisor; and
    // afterwards.
    let v = json(&dir, "person", &["item", "add", "first", "--phase", "P-1"]);
    assert_eq!((&v["item"], &v["phase"]), (&"I-2".into(), &"P-1".into()));
    ok(&dir, "kite", &["item", "add", "second", "--phase", "P-2"]);
    ok(
        &dir,
        "kite",
        &["item", "add", "also first", "--phase", "P-1"],
    );
    let v = json(&dir, "kite", &["item", "phase", "I-1", "P-2"]);
    assert_eq!((&v["item"], &v["phase"]), (&"I-1".into(), &"P-2".into()));
    let v = json(&dir, "kite", &["item", "phase", "I-1", "none"]);
    assert!(v["phase"].is_null(), "{v}");
    let err = refused(&dir, "kite", &["item", "phase", "I-1", "none"]);
    assert!(err.contains("I-1 already has no phase"), "{err}");

    // A seat that writes or audits does not plan phases.
    for seat in ["heron", "wren"] {
        let err = refused(&dir, seat, &["item", "phase", "I-1", "P-1"]);
        assert!(err.contains("does not plan phases"), "{err}");
        let err = refused(&dir, seat, &["item", "add", "mine", "--phase", "P-1"]);
        assert!(err.contains("does not plan phases"), "{err}");
    }
    // It adds to the backlog, with or without saying so.
    ok(&dir, "heron", &["item", "add", "found on the way"]);
    ok(&dir, "wren", &["item", "add", "a gap", "--phase", "none"]);
    assert!(phase_of(&dir, "I-5").is_null());
    assert!(phase_of(&dir, "I-6").is_null());

    // The summary shows what the next phase starts with, and counts
    // the rest.
    let text = ok(&dir, "person", &["summary"]);
    assert!(text.starts_with("no phase is open"), "{text}");
    assert!(
        text.contains("2 item(s) tagged for P-1, which has them when it opens:"),
        "{text}"
    );
    assert!(
        text.contains("I-2") && text.contains("I-4") && !text.contains("I-3 "),
        "{text}"
    );
    assert!(
        text.contains("4 item(s) and 0 open decision(s) outside the next phase are not shown"),
        "{text}"
    );
    let v = json(&dir, "person", &["summary"]);
    assert_eq!(v["next_phase"]["phase"], "P-1");
    let next: Vec<&str> = v["next_phase"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(next, ["I-2", "I-4"]);

    // Opening the phase brings them in and lists them.
    let v = json(&dir, "person", &["phase", "open", "the first"]);
    assert_eq!(v["brought_in"], serde_json::json!(["I-2", "I-4"]));
    assert_eq!(v["moved_on"], serde_json::json!([]));
    let ready = ok(&dir, "heron", &["ready"]);
    assert!(
        ready.contains("I-2") && ready.contains("I-4") && !ready.contains("I-3"),
        "{ready}"
    );
    ok(&dir, "heron", &["claim", "I-2"]);
    let err = refused(&dir, "heron", &["claim", "I-3"]);
    assert!(err.contains("I-3 is in P-2, not the open phase"), "{err}");
    let err = refused(&dir, "heron", &["claim", "I-1"]);
    assert!(err.contains("I-1 has no phase"), "{err}");

    // The tally counts what the phase opened with beside what was
    // planned in it, and keeps the count when a tag later moves.
    let tally = |dir: &Path| json(dir, "person", &["report"])["tally"].clone();
    assert_eq!(tally(&dir)["opened_with"], 2);
    assert_eq!(tally(&dir)["planned"], 0);
    ok(&dir, "person", &["item", "add", "found in flight"]);
    ok(&dir, "person", &["item", "phase", "I-4", "none"]);
    assert_eq!(
        (
            tally(&dir)["opened_with"].clone(),
            tally(&dir)["planned"].clone()
        ),
        (2.into(), 1.into())
    );
    let report = ok(&dir, "person", &["report"]);
    assert!(
        report.contains("P-1: opened with 2, 1 planned, 0 merged,"),
        "{report}"
    );

    // The log holds together and replays.
    ok(&dir, "person", &["verify"]);
}

#[test]
fn the_open_phases_scope_is_the_persons_and_a_closed_phase_takes_nothing() {
    let dir = project("scope");
    ok(&dir, "person", &["phase", "open", "one"]);
    ok(&dir, "person", &["item", "add", "in the phase"]);
    ok(&dir, "person", &["item", "add", "later", "--phase", "P-3"]);

    // With a phase open, a writing seat adds to it by name or to the
    // backlog, and to no other.
    ok(&dir, "heron", &["item", "add", "here", "--phase", "P-1"]);
    assert_eq!(phase_of(&dir, "I-3"), "P-1");
    let err = refused(&dir, "heron", &["item", "add", "there", "--phase", "P-2"]);
    assert!(err.contains("does not plan phases"), "{err}");

    // The advisor plans ahead, and leaves the open phase alone.
    let err = refused(&dir, "kite", &["item", "phase", "I-1", "P-2"]);
    assert!(
        err.contains("only the person moves I-1 into or out of the open phase"),
        "{err}"
    );
    let err = refused(&dir, "kite", &["item", "phase", "I-2", "P-1"]);
    assert!(err.contains("only the person"), "{err}");
    ok(&dir, "kite", &["item", "phase", "I-2", "P-2"]);

    // The person moves an item out of the phase and back.
    ok(&dir, "person", &["item", "phase", "I-1", "none"]);
    assert!(
        !ok(&dir, "heron", &["ready"]).contains("I-1"),
        "out of scope"
    );
    let moved = ok(&dir, "person", &["item", "phase", "I-1", "P-1"]);
    assert!(moved.contains("I-1 is in P-1, the open phase"), "{moved}");

    // Started work is not moved.
    ok(&dir, "heron", &["claim", "I-1"]);
    let err = refused(&dir, "person", &["item", "phase", "I-1", "none"]);
    assert!(err.contains("claimed"), "{err}");

    // Closing clears the tag of what has not merged, and says so.
    let closed = ok(&dir, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains("2 item(s) not merged have no phase now"),
        "{closed}"
    );
    assert!(phase_of(&dir, "I-1").is_null());
    assert!(phase_of(&dir, "I-3").is_null());
    assert_eq!(phase_of(&dir, "I-2"), "P-2");
    // Its tally still counts what was added while it was open.
    let report = ok(&dir, "person", &["report"]);
    assert!(
        report.contains("P-1: opened with 0, 3 planned, 0 merged, 3 carried over"),
        "{report}"
    );

    // A closed phase takes no more items.
    let err = refused(&dir, "person", &["item", "phase", "I-3", "P-1"]);
    assert!(
        err.contains("P-1 is closed and takes no more items"),
        "{err}"
    );
    let err = refused(&dir, "person", &["item", "add", "late", "--phase", "P-1"]);
    assert!(err.contains("P-1 is closed"), "{err}");
    let out = ris(&dir, "person", &["item", "phase", "I-3", "soon"]);
    assert!(!out.status.success(), "a tag is P-n or none");

    // The next phase opens with its tags moved on instead.
    let opened = ok(&dir, "person", &["phase", "open", "two", "--move-on"]);
    assert!(
        opened.contains("1 item(s) tagged for it moved on to P-3:"),
        "{opened}"
    );
    assert_eq!(phase_of(&dir, "I-2"), "P-3");
    assert_eq!(ok(&dir, "heron", &["ready"]).trim(), "nothing ready");
    // What it moved on, it did not open with.
    let v = json(&dir, "person", &["report"]);
    assert_eq!(v["tally"]["opened_with"], 0, "{v}");
    ok(&dir, "person", &["verify"]);
}

#[test]
fn a_retag_straight_after_the_open_does_not_shrink_what_the_phase_opened_with() {
    // With no docs item, nothing lands between the open and the
    // person's next command, so a retag of one item the phase opened
    // with follows PhaseOpened directly. The open moves every tagged
    // item or none, so a retag of some of them is the person's own.
    let dir = project("retag");
    ok(&dir, "person", &["item", "add", "one", "--phase", "P-1"]);
    ok(&dir, "person", &["item", "add", "two", "--phase", "P-1"]);
    ok(&dir, "person", &["phase", "open", "first"]);
    ok(&dir, "person", &["item", "phase", "I-1", "P-2"]);
    assert_eq!(phase_of(&dir, "I-1"), "P-2");
    let v = json(&dir, "person", &["report"]);
    assert_eq!(v["tally"]["opened_with"], 2, "{v}");
    ok(&dir, "person", &["verify"]);
}
