//! `ris seat pause` and `resume`: the person stops seats at the end of
//! what they hold, without typing into an agent's prompt.

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
fn a_paused_seat_finishes_its_item_and_takes_nothing_new() {
    let dir = std::env::temp_dir().join(format!("ris-pause-{}", std::process::id()));
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
    ok(&dir, None, &["phase", "open", "pause"]);
    ok(&dir, None, &["item", "add", "one", "--no-checks"]);
    ok(&dir, None, &["item", "add", "two", "--no-checks"]);
    let heron = Some("heron");
    ok(&dir, heron, &["claim", "I-1"]);

    let r = ris(&dir, heron, &["seat", "pause", "--all"]);
    assert_eq!(r.status.code(), Some(2), "an agent cannot pause seats");
    let r = ris(&dir, None, &["seat", "pause", "nobody"]);
    assert!(!r.status.success(), "an unknown seat is refused");

    let out = ok(&dir, None, &["seat", "pause", "--all"]);
    assert!(out.contains("heron finishes I-1"), "{out}");
    let status = ok(&dir, None, &["status"]);
    assert!(
        status
            .lines()
            .any(|l| l.starts_with("heron") && l.contains("holds I-1") && l.contains("paused")),
        "{status}"
    );

    // It finishes what it holds...
    fs::write(dir.join("note.txt"), "done\n").unwrap();
    ok(&dir, heron, &["done", "I-1", "--evidence", "note.txt"]);
    // ...and takes nothing new.
    let r = ris(&dir, heron, &["claim", "I-2"]);
    assert_eq!(r.status.code(), Some(2));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(err.contains("heron is paused by the person"), "{err}");
    assert!(err.contains("ris seat resume heron"), "{err}");

    ok(&dir, None, &["seat", "resume", "heron"]);
    ok(&dir, heron, &["claim", "I-2"]);
    let status = ok(&dir, None, &["status"]);
    assert!(
        status
            .lines()
            .any(|l| l.starts_with("wren") && l.contains("paused")),
        "wren stays paused: {status}"
    );
}
