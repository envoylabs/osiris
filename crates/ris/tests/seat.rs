//! `ris seat add`: the person adds a seat; a name is never reused.

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
fn the_person_adds_seats_and_a_name_in_the_log_is_never_reused() {
    let dir = std::env::temp_dir().join(format!("ris-seat-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
    let config = dir.join(".osiris/config.toml");

    let out = ok(&dir, None, &["seat", "add", "rook", "--role", "builder"]);
    assert!(out.contains("seat rook added"), "{out}");
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("# osiris configuration"), "comments kept");
    assert!(text.contains("[agents.rook]\nrole = \"builder\""), "{text}");

    // Only the person adds seats, and only once per name.
    assert!(
        !ris(
            &dir,
            Some("heron"),
            &["seat", "add", "kite", "--role", "lead"]
        )
        .status
        .success()
    );
    assert!(
        !ris(&dir, None, &["seat", "add", "rook", "--role", "lead"])
            .status
            .success()
    );
    assert!(
        !ris(&dir, None, &["seat", "add", "kite", "--role", "boss"])
            .status
            .success()
    );

    // The new seat works at once.
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replace(
            "denylist = []",
            "confirm_empty_denylist = true\ndenylist = []",
        ),
    )
    .unwrap();
    ok(&dir, None, &["phase", "open", "seats"]);
    ok(&dir, None, &["item", "add", "a task"]);
    ok(&dir, Some("rook"), &["claim", "I-1"]);

    // Remove rook from the config: the log still has it, so the name
    // cannot be given to a new seat.
    let without = fs::read_to_string(&config)
        .unwrap()
        .replace("[agents.rook]\nrole = \"builder\"\n", "");
    fs::write(&config, without).unwrap();
    let r = ris(&dir, None, &["seat", "add", "rook", "--role", "auditor"]);
    assert!(!r.status.success());
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("never given to another seat"),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
}
