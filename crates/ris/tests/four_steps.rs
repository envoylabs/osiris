//! The four steps: phase open, up, review, phase close. A substantive
//! decision raised while the phase runs sets its item aside and never
//! stops the seat; the person takes it at review with `ris decide`;
//! phase close merges what is merged, carries the rest over, and its
//! report counts the items planned, merged and carried over.

mod common;

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

fn data(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut a = vec!["--json"];
    a.extend_from_slice(args);
    let v: serde_json::Value = serde_json::from_str(&ok(dir, "person", &a)).unwrap();
    v["data"].clone()
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
    let dir = std::env::temp_dir().join(format!("ris-four-steps-{}", std::process::id()));
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

fn tally(v: &serde_json::Value) -> (u64, u64, u64) {
    let t = &v["tally"];
    (
        t["planned"].as_u64().unwrap(),
        t["merged"].as_u64().unwrap(),
        t["carried_over"].as_u64().unwrap(),
    )
}

#[test]
fn a_phase_runs_through_a_queued_decision_and_closes_with_its_tally() {
    let dir = setup();

    // Step one: open the phase and load it.
    ok(&dir, "person", &["phase", "open", "four steps"]);
    git(&dir, &["switch", "-q", "phase/P-1"]);
    for goal in ["first", "needs a judgement", "left for later"] {
        ok(&dir, "person", &["item", "add", goal, "--check", "ok"]);
    }

    // Step two, the seat's side: a substantive decision sets I-2 aside
    // and says to take another; the seat is not stopped.
    ok(&dir, "heron", &["claim", "I-2"]);
    let raised = ok(
        &dir,
        "heron",
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-2",
            "--question",
            "which way?",
            "--option",
            "a",
            "--option",
            "b",
            "--recommend",
            "0",
        ],
    );
    assert!(
        raised.contains("D-1 queued for the person's review; I-2 is set aside, take another"),
        "{raised}"
    );
    let refused = ris(&dir, "heron", &["decide", "D-1", "--choose", "0"]);
    assert!(!refused.status.success());
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(err.contains("at review, with `ris decide`"), "{err}");
    assert!(!err.contains("check-in"), "{err}");

    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    commit(&dir, "a.txt", "I-1");
    ok(&dir, "person", &["merge", "I-1"]);

    // While open, the report counts what would carry over.
    let report = ok(&dir, "person", &["report"]);
    assert!(
        report.contains("P-1: 3 planned, 1 merged, 2 carry over if closed now"),
        "{report}"
    );
    assert_eq!(tally(&data(&dir, &["report"])), (3, 1, 2));

    // Step four, without step three: the queued decision is still open,
    // and close carries it and its item over.
    let closed = data(&dir, &["phase", "close"]);
    assert_eq!(tally(&closed["phase"]), (3, 1, 2));
    let report = ok(&dir, "person", &["report"]);
    assert!(
        report.contains("P-1: 3 planned, 1 merged, 2 carried over"),
        "{report}"
    );

    // The next phase: review takes the decision, which frees I-2, and
    // its merge counts in the phase where it merged.
    ok(&dir, "person", &["phase", "open", "the next phase"]);
    git(&dir, &["switch", "-q", "phase/P-2"]);
    ok(&dir, "person", &["item", "add", "fourth", "--check", "ok"]);
    ok(&dir, "person", &["decide", "D-1", "--choose", "0"]);
    ok(&dir, "heron", &["claim", "I-2"]);
    ok(&dir, "heron", &["done", "I-2"]);
    commit(&dir, "b.txt", "I-2");
    ok(&dir, "person", &["merge", "I-2"]);
    // Planned: I-4. Merged: I-2, carried in. Open: I-3 and I-4.
    assert_eq!(tally(&data(&dir, &["report"])), (1, 1, 2));
}
