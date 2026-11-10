//! Who is acting: `RIS_AGENT=person`, or a configured seat. Unset is
//! refused, never taken to mean the person, so an agent that loses its
//! seat variable cannot act with the person's powers.

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

#[test]
fn an_unset_ris_agent_is_refused_and_person_is_reserved() {
    let dir = std::env::temp_dir().join(format!("ris-actor-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Init needs no actor: there is no project to act on yet.
    assert!(ris(&dir, None, &["init"]).status.success());
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();

    // Unset: refused, for reading as for writing, naming both ways out.
    for args in [
        &["phase", "open", "x"][..],
        &["ready"],
        &["decide", "D-1", "--choose", "0", "--rationale", "r"],
    ] {
        let r = ris(&dir, None, args);
        assert_eq!(r.status.code(), Some(2), "ris {}", args.join(" "));
        let err = String::from_utf8_lossy(&r.stderr);
        assert!(err.contains("RIS_AGENT is not set"), "{err}");
        assert!(err.contains("RIS_AGENT=person"), "{err}");
        assert!(err.contains("heron"), "the seats are named: {err}");
    }
    // An empty value is unset.
    assert_eq!(ris(&dir, Some(" "), &["ready"]).status.code(), Some(2));
    // In JSON, the refusal has its own kind.
    let r = ris(&dir, None, &["--json", "ready"]);
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "no_actor");
    assert!(r.stderr.is_empty());

    // The person, said explicitly; a seat; anything else is refused.
    assert!(
        ris(&dir, Some("person"), &["phase", "open", "x"])
            .status
            .success()
    );
    assert!(ris(&dir, Some("heron"), &["ready"]).status.success());
    assert_eq!(ris(&dir, Some("nobody"), &["ready"]).status.code(), Some(1));

    // No seat may be called person, by command or by hand.
    let r = ris(
        &dir,
        Some("person"),
        &["seat", "add", "person", "--role", "builder"],
    );
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("cannot be a seat"));
    let text = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!("{text}\n[agents.person]\nrole = \"lead\"\n"),
    )
    .unwrap();
    let r = ris(&dir, Some("person"), &["ready"]);
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("cannot be a seat"));
}
