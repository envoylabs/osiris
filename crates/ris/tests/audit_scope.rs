//! `ris audit` is the open phase's review queue: an earlier phase's
//! unreviewed items are left out unless `--all` is asked for, so the
//! auditor's work is bounded by the phase. Recording a review is not
//! limited by phase.

use std::fs;
use std::path::Path;
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

fn queue(dir: &Path, agent: &str, args: &[&str]) -> Vec<String> {
    let mut all = vec!["--json", "audit"];
    all.extend_from_slice(args);
    let v: Value = serde_json::from_str(&ok(dir, agent, &all)).unwrap();
    v["data"]["queue"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_audit_queue_is_the_open_phases_unless_all_is_asked_for() {
    let dir = std::env::temp_dir().join(format!("ris-audit-scope-{}", std::process::id()));
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
    // `init` seats wren as the auditor.
    assert!(ok(&dir, "person", &["status"]).contains("wren"));
    fs::write(dir.join("note.txt"), "done\n").unwrap();

    // P-1 finishes an item nobody reviews.
    ok(&dir, "person", &["phase", "open", "first"]);
    ok(&dir, "person", &["item", "add", "old work", "--no-checks"]);
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1", "--evidence", "note.txt"]);
    ok(&dir, "person", &["phase", "close"]);

    // P-2 finishes one of its own.
    ok(&dir, "person", &["phase", "open", "second"]);
    ok(&dir, "person", &["item", "add", "new work", "--no-checks"]);
    ok(&dir, "heron", &["claim", "I-2"]);
    ok(&dir, "heron", &["done", "I-2", "--evidence", "note.txt"]);

    // The auditor's queue, and the person's, hold only the open phase's.
    assert_eq!(queue(&dir, "wren", &[]), ["I-2"]);
    assert_eq!(queue(&dir, "person", &[]), ["I-2"]);
    let text = ok(&dir, "wren", &["audit"]);
    assert!(
        text.contains("new work") && !text.contains("old work"),
        "{text}"
    );

    // --all shows every phase's.
    assert_eq!(queue(&dir, "wren", &["--all"]), ["I-1", "I-2"]);
    assert_eq!(queue(&dir, "person", &["--all"]), ["I-1", "I-2"]);

    // --all names a queue, not a review: it does not go with an item.
    let r = ris(&dir, "wren", &["audit", "I-2", "--all"]);
    assert!(!r.status.success());

    // A review is recorded whatever the item's phase.
    ok(&dir, "wren", &["audit", "I-1"]);
    ok(&dir, "wren", &["audit", "I-2"]);
    assert!(queue(&dir, "wren", &["--all"]).is_empty());
    assert!(ok(&dir, "wren", &["audit"]).contains("nothing to review"));
}
