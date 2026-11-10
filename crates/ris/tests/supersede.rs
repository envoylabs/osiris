//! `ris item supersede`: the person replaces an item nobody has started;
//! the old one leaves `ris ready` for good and what depended on it waits
//! on the replacement.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
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

#[test]
fn a_superseded_item_leaves_ready_and_its_dependents_wait_on_the_replacement() {
    let dir = std::env::temp_dir().join(format!("ris-supersede-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    ok(&dir, None, &["phase", "open", "supersede"]);
    ok(&dir, None, &["item", "add", "the old goal"]);
    ok(&dir, None, &["item", "add", "the new goal"]);
    ok(
        &dir,
        None,
        &["item", "add", "built on it", "--depends", "I-1"],
    );

    let r = ris(
        &dir,
        Some("heron"),
        &["item", "supersede", "I-1", "--by", "I-2"],
    );
    assert_eq!(r.status.code(), Some(2), "an agent is refused");
    assert!(String::from_utf8_lossy(&r.stderr).contains("only the person"));

    assert_eq!(
        ok(&dir, None, &["item", "supersede", "i1", "--by", "I2"]).trim(),
        "I-1 superseded by I-2"
    );
    let ready = ok(&dir, None, &["ready"]);
    assert!(!ready.contains("the old goal"), "{ready}");
    assert!(ready.contains("I-2"), "{ready}");
    assert!(!ready.contains("I-3"), "I-3 now waits on I-2: {ready}");
    assert!(ok(&dir, None, &["show", "I-1"]).contains("superseded by: I-2"));
    assert!(ok(&dir, None, &["show", "I-3"]).contains("waiting on I-2"));
    assert!(ok(&dir, None, &["report"]).contains("(by I-2)"));

    // Final: it cannot be claimed or superseded again, and nothing can
    // be replaced by it.
    assert_eq!(
        ris(&dir, Some("heron"), &["claim", "I-1"]).status.code(),
        Some(2)
    );
    assert_eq!(
        ris(&dir, None, &["item", "supersede", "I-1", "--by", "I-3"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        ris(&dir, None, &["item", "supersede", "I-3", "--by", "I-1"])
            .status
            .code(),
        Some(2)
    );
}
