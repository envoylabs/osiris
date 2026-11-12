//! A seat works only the open phase's items: `ris ready` shows it no
//! other, and `ris claim` refuses an item in the backlog until the
//! person brings it in. The person's `ris ready` puts the open phase's
//! items before the backlog carried over from earlier phases.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn ids(ready: &str) -> Vec<&str> {
    ready
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect()
}

/// A project with the template's seats, I-1 left in P-1's backlog and I-2
/// added in the open P-2.
fn project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-ready-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, "person", &["init"]);
    let config = dir.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap().replace(
        "denylist = []",
        "denylist = [\"example client\"]\ndocs_item = false",
    );
    fs::write(&config, c).unwrap();
    ok(&dir, "person", &["phase", "open", "first"]);
    ok(&dir, "person", &["item", "add", "old work"]);
    ok(&dir, "person", &["phase", "close"]);
    ok(&dir, "person", &["phase", "open", "second"]);
    ok(&dir, "person", &["item", "add", "this phase's work"]);
    dir
}

#[test]
fn the_persons_ready_puts_the_phases_own_items_before_the_backlog() {
    let dir = project("person");
    let ready = ok(&dir, "person", &["ready"]);
    assert_eq!(ids(&ready), ["I-2", "I-1"], "{ready}");
}

#[test]
fn a_seat_sees_and_claims_only_the_open_phases_items() {
    let dir = project("seat");
    let ready = ok(&dir, "heron", &["ready"]);
    assert_eq!(ids(&ready), ["I-2"], "{ready}");

    let out = ris(&dir, "heron", &["claim", "I-1"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("I-1 is in P-1, not the open phase"), "{err}");
    assert!(err.contains("ris item bring-in I-1"), "{err}");

    // Only the person brings an item in.
    let out = ris(&dir, "heron", &["item", "bring-in", "I-1"]);
    assert_eq!(out.status.code(), Some(2));

    let brought = ok(&dir, "person", &["--json", "item", "bring-in", "I-1"]);
    let v: serde_json::Value = serde_json::from_str(&brought).unwrap();
    assert_eq!(v["data"]["item"], "I-1");
    assert_eq!(v["data"]["phase"], "P-2");

    let ready = ok(&dir, "heron", &["ready"]);
    assert_eq!(ids(&ready), ["I-1", "I-2"], "{ready}");
    ok(&dir, "heron", &["claim", "I-1"]);

    // The log records the move, and the chain still verifies.
    let log = ok(&dir, "person", &["--json", "log"]);
    assert!(
        log.lines().any(|l| l.contains("\"item_scoped\"")
            && l.contains("\"I-1\"")
            && l.contains("\"P-2\"")),
        "{log}"
    );
    ok(&dir, "person", &["verify"]);
}
