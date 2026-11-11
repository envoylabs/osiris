//! `ris decide` with no decision walks the person's queue of open
//! substantive decisions; `ris decide --json` gives the queue. Stdin is
//! not a terminal here, so each prompt reads a line whose first
//! character is the key.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn ris_in(dir: &Path, agent: &str, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn ok(dir: &Path, agent: &str, args: &[&str], stdin: &str) -> String {
    let out = ris_in(dir, agent, args, stdin);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn raise(dir: &Path, question: &str) {
    ok(
        dir,
        "heron",
        &[
            "decision",
            "raise",
            "--substantive",
            "--question",
            question,
            "--option",
            "first",
            "--option",
            "second",
            "--recommend",
            "0",
        ],
        "",
    );
}

fn state(dir: &Path, id: &str) -> serde_json::Value {
    let out = ok(dir, "person", &["--json", "show", id], "");
    serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"].clone()
}

#[test]
fn the_person_walks_the_queue_one_decision_at_a_time() {
    let dir = std::env::temp_dir().join(format!("ris-decide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, "person", &["init"], "");
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
    ok(&dir, "person", &["phase", "open", "decide"], "");
    raise(&dir, "Which engine?");
    raise(&dir, "Which format?");
    raise(&dir, "Which name?");
    ok(
        &dir,
        "heron",
        &[
            "decision",
            "explain",
            "D-1",
            "--summary",
            "Where the log lives",
            "--pro",
            "0: nothing to install",
            "--con",
            "1: a native build",
            "--why",
            "One file to back up",
        ],
        "",
    );

    // Only the person walks the queue.
    let agent = ris_in(&dir, "heron", &["decide"], "0\n\n");
    assert!(!agent.status.success());
    assert!(
        String::from_utf8_lossy(&agent.stderr).contains("the person's"),
        "{agent:?}"
    );

    // The queue as JSON: each entry is the eli5 document.
    let q: serde_json::Value =
        serde_json::from_str(&ok(&dir, "person", &["--json", "decide"], "")).unwrap();
    let queue = q["data"]["queue"].as_array().unwrap();
    let ids: Vec<&str> = queue
        .iter()
        .map(|e| e["decision"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["D-1", "D-2", "D-3"], "{q}");
    assert_eq!(
        queue[0]["explanation"]["summary"], "Where the log lives",
        "{q}"
    );
    assert!(queue[1]["explanation"].is_null(), "{q}");

    // Pick option 1 for D-1 with a rationale, skip D-2 (an unknown key
    // and an out-of-range digit are read past), and input ends at D-3:
    // that quits, keeping D-1.
    let out = ok(
        &dir,
        "person",
        &["decide"],
        "x\n7\n1\nnative is fine here\ns\n",
    );
    // Each screen is the case, then the prompt; not the eli5 footer.
    assert!(out.contains("Where the log lives"), "{out}");
    assert!(out.contains("+ nothing to install"), "{out}");
    assert!(out.contains("D-1: pick 0-1, s to skip, q to quit"), "{out}");
    assert!(!out.contains("answer:  ris decide"), "{out}");
    assert!(out.contains("D-1 taken: option 1"), "{out}");
    assert!(out.contains("Which format?"), "{out}");
    assert!(out.contains("Which name?"), "{out}");
    assert!(out.contains("still open: D-2, D-3"), "{out}");
    let d1 = state(&dir, "D-1");
    assert_eq!(d1["state"], "taken", "{d1}");
    assert_eq!(d1["chosen"], 1, "{d1}");
    assert_eq!(d1["rationale"], "native is fine here", "{d1}");
    assert_eq!(state(&dir, "D-2")["state"], "open");

    // q quits at once and takes nothing.
    let out = ok(&dir, "person", &["decide"], "q\n");
    assert!(out.contains("taken: none"), "{out}");
    assert!(!out.contains("Which name?"), "{out}");

    // Enter skips the rationale; the walk takes both that remain.
    let out = ok(&dir, "person", &["decide"], "0\n\n1\n\n");
    assert!(out.contains("D-2 taken: option 0"), "{out}");
    assert!(out.contains("D-3 taken: option 1"), "{out}");
    let d2 = state(&dir, "D-2");
    assert_eq!(d2["state"], "taken", "{d2}");
    assert_eq!(d2["rationale"], "", "{d2}");

    let out = ok(&dir, "person", &["decide"], "");
    assert!(out.contains("no open substantive decisions"), "{out}");
    let q: serde_json::Value =
        serde_json::from_str(&ok(&dir, "person", &["--json", "decide"], "")).unwrap();
    assert_eq!(q["data"]["queue"], serde_json::json!([]));
}

#[test]
fn a_named_decision_needs_an_answer_and_an_answer_needs_a_decision() {
    let dir = std::env::temp_dir().join(format!("ris-decide-args-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, "person", &["init"], "");
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
    ok(&dir, "person", &["phase", "open", "decide"], "");
    raise(&dir, "Which engine?");

    let bare = ris_in(&dir, "person", &["decide", "D-1"], "");
    assert_eq!(bare.status.code(), Some(1), "{bare:?}");
    assert!(String::from_utf8_lossy(&bare.stderr).contains("--choose"));
    let orphan = ris_in(&dir, "person", &["decide", "--choose", "0"], "");
    assert!(!orphan.status.success(), "{orphan:?}");

    // The rationale is optional on a named decision too.
    ok(&dir, "person", &["decide", "D-1", "--choose", "0"], "");
    assert_eq!(state(&dir, "D-1")["rationale"], "");
}
