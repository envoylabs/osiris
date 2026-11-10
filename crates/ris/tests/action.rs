//! Person actions: something only the person can do, raised by anyone,
//! done only by the person, blocking the items that wait for it and
//! informing those that proceed without it.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

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
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent.unwrap_or("person"))
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    serde_json::from_slice::<Value>(&ris(dir, None, &all).stdout).unwrap()["data"].clone()
}

#[test]
fn an_action_blocks_and_informs_and_only_the_person_does_it() {
    let dir = std::env::temp_dir().join(format!("ris-action-{}", std::process::id()));
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
        "denylist = [\"example client\"]\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    ok(&dir, None, &["phase", "open", "actions"]);
    ok(&dir, None, &["item", "add", "the runner"]);
    ok(&dir, None, &["item", "add", "go"]);
    let heron = Some("heron");

    // An agent raises it: I-1 waits, I-2 proceeds.
    let out = ok(
        &dir,
        heron,
        &[
            "action",
            "raise",
            "add a builder seat",
            "--done-when",
            "ris status lists it",
            "--blocks",
            "I-1",
            "--informs",
            "i2",
        ],
    );
    assert!(
        out.contains("A-1 raised for the person; I-1 set aside"),
        "{out}"
    );
    assert_eq!(
        json(&dir, &["show", "I-1"])["blocked_on"],
        serde_json::json!([{ "kind": "action", "id": "A-1" }])
    );
    let inbox = ok(&dir, None, &["inbox"]);
    assert!(inbox.contains("1 action(s) for you"), "{inbox}");
    assert!(inbox.contains("A-1 add a builder seat"), "{inbox}");
    assert_eq!(json(&dir, &["inbox"])["actions"][0]["id"], "A-1");
    let edges = json(&dir, &["summary"])["graph"]["edges"].clone();
    assert_eq!(
        edges,
        serde_json::json!([
            { "from": "A-1", "to": "I-1", "kind": "blocks" },
            { "from": "A-1", "to": "I-2", "kind": "informs" },
        ])
    );
    let brief = ok(&dir, heron, &["claim", "I-2"]);
    assert!(
        brief.contains("A-1 (informs) add a builder seat → open"),
        "{brief}"
    );

    // I-2 goes ahead and merges before the person has done A-1.
    ok(&dir, heron, &["done", "I-2"]);
    fs::write(dir.join("go.txt"), "go\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "commit",
            "-q",
            "-m",
            "feat: go",
            "-m",
            "Item: I-2\nAgent: heron",
        ],
    );
    ok(&dir, None, &["merge", "I-2"]);
    let report = ok(&dir, None, &["report"]);
    assert!(
        report.contains("I-2 merged before A-1 was done"),
        "{report}"
    );

    // Only the person does it, and only with a result.
    let r = ris(&dir, heron, &["action", "done", "A-1", "--note", "added"]);
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("only the person does A-1"));
    let r = ris(&dir, None, &["action", "done", "A-1"]);
    assert_eq!(r.status.code(), Some(2));
    let out = ok(
        &dir,
        None,
        &["action", "done", "a1", "--note", "rook added"],
    );
    assert!(out.contains("A-1 done; ready again: I-1"), "{out}");
    assert!(ok(&dir, None, &["show", "A-1"]).contains("done: rook added"));
    assert_eq!(json(&dir, &["show", "A-1"])["state"], "done");
    // Still flagged: the merge happened before it was done.
    assert_eq!(
        json(&dir, &["report"])["merged_ahead"],
        serde_json::json!([{ "item": "I-2", "action": "A-1" }])
    );
    ok(&dir, heron, &["claim", "I-1"]);
    ok(&dir, None, &["verify"]);
}
