//! `ris ready` puts the open phase's own items before the backlog
//! carried over from earlier phases, so a seat taking the first item
//! works on the phase.

use std::fs;
use std::path::Path;
use std::process::Command;

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_phases_own_items_come_before_carried_ones() {
    let dir = std::env::temp_dir().join(format!("ris-ready-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, &["init"]);
    let config = dir.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap().replace(
        "denylist = []",
        "denylist = [\"example client\"]\ndocs_item = false",
    );
    fs::write(&config, c).unwrap();
    ok(&dir, &["phase", "open", "first"]);
    ok(&dir, &["item", "add", "old work"]);
    ok(&dir, &["phase", "close"]);
    ok(&dir, &["phase", "open", "second"]);
    ok(&dir, &["item", "add", "this phase's work"]);
    let ready = ok(&dir, &["ready"]);
    let ids: Vec<&str> = ready
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(ids, ["I-2", "I-1"], "{ready}");
}
