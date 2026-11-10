//! `ris export`: one JSONL file per closed phase, scanned first, and
//! enough to verify and replay elsewhere.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use ris_core::event::{Event, verify};
use ris_core::state::State;

fn ris(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) {
    let out = ris(dir, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn closed_phases_export_verify_and_replay() {
    let base = std::env::temp_dir().join(format!("ris-export-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    ok(&repo, &["init"]);
    let config = repo.join(".osiris/config.toml");
    // The denylist gate refuses the first phase until it is filled.
    let r = ris(&repo, &["phase", "open", "first"]);
    assert_eq!(r.status.code(), Some(1));
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace("denylist = []", "denylist = [\"example client\"]"),
    )
    .unwrap();
    ok(&repo, &["phase", "open", "first"]);
    ok(&repo, &["item", "add", "one"]);
    ok(&repo, &["phase", "close"]);
    ok(&repo, &["phase", "open", "second"]);
    ok(&repo, &["item", "add", "the acme thing"]);
    ok(&repo, &["phase", "close"]);
    ok(&repo, &["phase", "open", "third, still open"]);

    let out = base.join("out");
    ok(&repo, &["export", out.to_str().unwrap()]);
    let mut log: Vec<Event> = Vec::new();
    for name in ["P-1.jsonl", "P-2.jsonl"] {
        for line in fs::read_to_string(out.join(name)).unwrap().lines() {
            log.push(serde_json::from_str(line).unwrap());
        }
    }
    assert!(
        !out.join("P-3.jsonl").exists(),
        "an open phase is not exported"
    );
    // Another machine can verify the chain and replay the state.
    let v = verify(&log).unwrap();
    assert_eq!(v.len, 6);
    let state = State::fold(&log).unwrap();
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.phases.len(), 2);

    // Exporting again is a no-op; a file that changed is refused.
    ok(&repo, &["export", out.to_str().unwrap()]);
    fs::write(out.join("P-1.jsonl"), "tampered\n").unwrap();
    let r = ris(&repo, &["export", out.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stderr).contains("P-1.jsonl exists and differs"));

    // A denylist that grew since the text went in stops the export
    // before anything is written.
    let c = fs::read_to_string(&config)
        .unwrap()
        .replace("denylist = [\"example client\"]", "denylist = [\"acme\"]");
    fs::write(&config, c).unwrap();
    let fresh = base.join("fresh");
    let r = ris(&repo, &["export", fresh.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("`goal`") && !err.contains("acme thing"),
        "{err}"
    );
    assert!(!fresh.exists(), "nothing was written");
}
