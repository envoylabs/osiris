//! `ris seat start`: the runner starts a seat's agent in its own
//! directory, as that seat, refused rather than asked outside the
//! role's allowlist, sandboxed, and records its start and stop. A fake
//! agent program on PATH stands in for the agent tool and prints what
//! it was given.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

fn ris(dir: &Path, bin: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent.unwrap_or("person"))
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .env("PATH", path)
        .output()
        .unwrap()
}

#[test]
fn a_seat_runs_in_its_directory_as_itself_within_its_role() {
    let base = std::env::temp_dir().join(format!("ris-runner-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let bin = base.join("bin");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // The fake agent: prints who and where it is and every argument,
    // then exits 3 so the status is seen to be carried through.
    let fake = bin.join("fake-agent");
    fs::write(
        &fake,
        "#!/bin/sh\nprintf 'AGENT %s\\n' \"$RIS_AGENT\"\nprintf 'DIR %s\\n' \"$(basename \"$PWD\")\"\n\
         printf 'RIS %s\\n' \"$(command -v ris)\"\n\
         for a in \"$@\"; do printf 'ARG %s\\n' \"$a\"; done\nexit 3\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("README"), "base\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let r = |agent: Option<&str>, args: &[&str]| ris(&repo, &bin, agent, args);
    assert!(r(None, &["init", "--local"]).status.success());
    fs::write(
        repo.join(".osiris/checks.toml"),
        "[checks.test]\ncommand = \"cargo test --workspace\"\n",
    )
    .unwrap();
    fs::write(
        repo.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\n\
         [runner]\nprogram = \"fake-agent\"\nwrite = [\"~/.cargo\"]\n\
         [runner.allow]\nbuilder = [\"Bash(cargo *)\"]\n\
         [agents.rook]\nrole = \"builder\"\nmodel = \"some-model\"\n\
         [agents.wren]\nrole = \"auditor\"\n",
    )
    .unwrap();
    assert!(r(None, &["phase", "open", "runner"]).status.success());

    // A builder needs its worktree first.
    let out = r(None, &["seat", "start", "rook"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ris worktree rook"));
    assert!(r(None, &["worktree", "rook"]).status.success());

    // Only the person starts a seat.
    assert_eq!(
        r(Some("rook"), &["seat", "start", "rook"]).status.code(),
        Some(2)
    );

    let out = r(None, &["seat", "start", "rook"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let args: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("ARG "))
        .collect();
    let after = |flag: &str| {
        args.iter()
            .position(|a| *a == flag)
            .map(|i| args[i + 1])
            .unwrap_or_else(|| panic!("{flag} missing: {text}"))
    };
    assert!(text.contains("AGENT rook"), "{text}");
    assert!(
        text.contains("DIR repo-rook"),
        "it runs in its worktree: {text}"
    );
    // `ris` on the seat's PATH is the ris that started it, so the
    // `Bash(ris *)` rule matches the seat's loop commands.
    let found = text
        .lines()
        .find_map(|l| l.strip_prefix("RIS "))
        .unwrap_or_default();
    assert!(!found.is_empty(), "ris is on the seat's PATH: {text}");
    assert_eq!(
        fs::canonicalize(found).unwrap(),
        fs::canonicalize(env!("CARGO_BIN_EXE_ris")).unwrap()
    );
    assert_eq!(after("--permission-mode"), "dontAsk");
    assert_eq!(after("--permission-prompts"), "none");
    assert_eq!(after("--model"), "some-model");
    let allow = after("--allowedTools");
    for t in [
        "Edit",
        "Bash(git *)",
        "Bash(ris *)",
        "Bash(cargo test --workspace)",
        "Bash(cargo *)",
    ] {
        assert!(allow.split(',').any(|a| a == t), "{t} in {allow}");
    }
    let settings: Value = serde_json::from_str(after("--settings")).unwrap();
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    let write: Vec<&str> = settings["sandbox"]["filesystem"]["allowWrite"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(write.iter().any(|w| w.ends_with("/.osiris")), "{write:?}");
    assert!(write.iter().any(|w| w.ends_with(".git")), "{write:?}");
    assert!(write.contains(&"~/.cargo"), "{write:?}");
    // The files that set its limits are denied, at both levels.
    let deny_write: Vec<&str> = settings["sandbox"]["filesystem"]["denyWrite"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for f in [
        ".osiris/config.toml",
        ".osiris/checks.toml",
        "repo-rook/CLAUDE.local.md",
    ] {
        assert!(
            deny_write.iter().any(|w| w.ends_with(f)),
            "{f} in {deny_write:?}"
        );
    }
    let denied = after("--disallowedTools");
    assert!(
        denied
            .split(',')
            .any(|d| d.starts_with("Edit(//") && d.ends_with("/.osiris/config.toml)")),
        "{denied}"
    );
    assert!(args.iter().any(|a| a.contains("RIS_AGENT is set to rook")));
    assert!(text.contains("rook stopped (exit 3)"), "{text}");

    // The start and stop are in the log, with the binding as text.
    let log = String::from_utf8(r(None, &["log"]).stdout).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let started = events
        .iter()
        .find(|e| e["structure"]["event"] == "run_started")
        .unwrap();
    assert_eq!(started["structure"]["seat"], "rook");
    assert_eq!(started["text"]["runner"], "claude");
    assert_eq!(started["text"]["model"], "some-model");
    let stopped = events
        .iter()
        .find(|e| e["structure"]["event"] == "run_stopped")
        .unwrap();
    assert_eq!(stopped["structure"]["status"], 3);
    // The output is kept, under .osiris/runs/.
    let runs: Vec<_> = fs::read_dir(repo.join(".osiris/runs")).unwrap().collect();
    assert_eq!(runs.len(), 1);
    let kept = fs::read_to_string(runs[0].as_ref().unwrap().path()).unwrap();
    assert!(kept.contains("AGENT rook"));

    // An auditor runs in the main checkout and cannot edit.
    let out = r(None, &["seat", "start", "wren", "--quiet"]);
    assert!(out.status.success());
    let kept: String = fs::read_dir(repo.join(".osiris/runs"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .filter(|s| s.contains("AGENT wren"))
        .collect();
    assert!(kept.contains("DIR repo"), "{kept}");
    let allow = kept
        .lines()
        .skip_while(|l| *l != "ARG --allowedTools")
        .nth(1)
        .unwrap();
    assert!(
        !allow.split(',').any(|a| a == "ARG Edit" || a == "Edit"),
        "{allow}"
    );
    assert!(allow.contains("Bash(git diff *)"), "{allow}");

    // A tool that is not there: refused, and not left recorded as running.
    let config = repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(&config, c.replace("fake-agent", "no-such-agent-tool")).unwrap();
    let out = r(None, &["seat", "start", "rook"]);
    assert!(!out.status.success());
    let status = String::from_utf8(r(None, &["status"]).stdout).unwrap();
    assert!(!status.contains("running"), "{status}");
}
