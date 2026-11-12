//! A machine with no git identity configured, as on a CI runner: the
//! closing merge, a commit ris makes itself, is refused as a missing
//! identity, with the commands that set one, and never reported as a
//! conflict with the main line. The phase stays open and the main line
//! does not move.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// `ris` as the person with git's identity off: no global or system
/// config, nothing in the environment, and `user.useConfigOnly` so git
/// does not guess one from the machine.
fn ris_without_identity(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.useConfigOnly")
        .env("GIT_CONFIG_VALUE_0", "true");
    for (key, _) in common::GIT_IDENTITY {
        cmd.env_remove(key);
    }
    cmd.env_remove("EMAIL");
    cmd.output().unwrap()
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

fn setup() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-git-identity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, "person", &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        dir.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_close_with_no_git_identity_is_refused_as_that() {
    let dir = setup();
    ok(&dir, "person", &["phase", "open", "identity"]);
    git(&dir, &["switch", "-q", "phase/P-1"]);
    ok(&dir, "person", &["item", "add", "first", "--check", "ok"]);
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "commit",
            "-q",
            "-m",
            "feat: a",
            "-m",
            "Item: I-1\nAgent: heron",
        ],
    );
    ok(&dir, "person", &["merge", "I-1"]);
    let main_before = git(&dir, &["rev-parse", "main"]);

    let out = ris_without_identity(&dir, &["phase", "close"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("refused: git has no identity to commit with here"),
        "{err}"
    );
    assert!(
        err.contains("the closing merge of phase/P-1 into main"),
        "{err}"
    );
    assert!(err.contains("git config user.name"), "{err}");
    assert!(err.contains("git config user.email"), "{err}");
    assert!(!err.contains("conflict"), "{err}");

    // The same in the --json envelope: the message and the next step.
    let out = ris_without_identity(&dir, &["--json", "phase", "close"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("git has no identity to commit with here"),
        "{v}"
    );
    assert!(!message.contains("conflict"), "{v}");
    let next = v["error"]["next"].as_str().unwrap();
    assert!(next.contains("git config user.email"), "{v}");

    // Nothing moved: the main line is where it was and the phase is open.
    assert_eq!(git(&dir, &["rev-parse", "main"]), main_before);
    let report = ok(&dir, "person", &["report"]);
    assert!(report.contains("P-1 open"), "{report}");

    // With an identity, the same close goes through.
    ok(&dir, "person", &["phase", "close"]);
    assert_ne!(git(&dir, &["rev-parse", "main"]), main_before);
}
