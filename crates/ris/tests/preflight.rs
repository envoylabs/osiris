//! Before any seat starts, `ris up` checks that the writing seats can
//! run the project's checks: the build tool a check names may write its
//! caches and reach its registry, and its program is on PATH. Phase
//! open says the same, so the configuration is fixed first. A seat that
//! finds a missing permission reports it with `ris seat fault`, which
//! `ris status` shows until the seat's next run, not as a decision.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Rig {
    repo: PathBuf,
    bin: PathBuf,
}

fn script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Rig {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("ris-preflight-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let bin = base.join("bin");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&bin).unwrap();
        // The agent tool and the build tool, as stand-ins on PATH.
        script(&bin.join("fake-agent"), "#!/bin/sh\nexit 0\n");
        script(&bin.join("cargo"), "#!/bin/sh\nexit 0\n");
        let rig = Self { repo, bin };
        rig.git(&["init", "-q", "-b", "main"]);
        fs::write(rig.repo.join("README"), "base\n").unwrap();
        rig.git(&["add", "-A"]);
        rig.git(&["commit", "-q", "-m", "base"]);
        rig.ok(None, &["init", "--local"]);
        fs::write(
            rig.repo.join(".osiris/checks.toml"),
            "[checks.test]\ncommand = \"cargo test --workspace\"\n",
        )
        .unwrap();
        rig.config("");
        rig
    }

    fn config(&self, runner: &str) {
        fs::write(
            self.repo.join(".osiris/config.toml"),
            format!(
                "denylist = [\"example client\"]\ndocs_item = false\n\
                 [runner]\nprogram = \"fake-agent\"\n{runner}\n\
                 [agents.rook]\nrole = \"builder\"\n"
            ),
        )
        .unwrap();
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
            .args(args)
            .current_dir(&self.repo)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {}", args.join(" "));
    }

    fn ris(&self, agent: Option<&str>, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_ris"))
            .args(args)
            .current_dir(&self.repo)
            .env("RIS_AGENT", agent.unwrap_or("person"))
            .env_remove("RIS_ROOT")
            .env("NO_COLOR", "1")
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn ok(&self, agent: Option<&str>, args: &[&str]) -> String {
        let out = self.ris(agent, args);
        assert!(
            out.status.success(),
            "ris {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn status(&self) -> serde_json::Value {
        let v: serde_json::Value =
            serde_json::from_str(&self.ok(None, &["--json", "status"])).unwrap();
        v["data"]["seats"][0].clone()
    }

    /// Wait until the seat's run has recorded its stop.
    fn stopped(&self) {
        let start = Instant::now();
        while self.status()["running"] == true {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "run never stopped"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
fn up_starts_no_seat_until_the_checks_can_run_and_a_fault_shows_in_status() {
    let rig = Rig::new();

    // Phase open names what is missing, and the lines that allow it.
    let opened = rig.ok(None, &["phase", "open", "preflight"]);
    assert!(
        opened.contains("write = [\"~/.cargo/registry\", \"~/.cargo/git\"]"),
        "{opened}"
    );
    assert!(
        opened.contains("network = [\"index.crates.io\", \"static.crates.io\"]"),
        "{opened}"
    );

    // Up refuses, and nothing starts: no worktree, no run.
    let refused = rig.ris(None, &["up"]);
    assert!(!refused.status.success());
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(err.contains("cargo fetches from index.crates.io"), "{err}");
    assert!(err.contains("preflight = false"), "{err}");
    let seat = rig.status();
    assert_eq!(seat["running"], false);
    assert!(seat["worktree"].is_null(), "{seat}");

    // Allowed, up goes ahead.
    rig.config("write = [\"~/.cargo\"]\nnetwork = [\"*.crates.io\"]");
    rig.ok(None, &["up"]);
    rig.stopped();

    // A seat reports a missing permission as a setup fault; the person
    // may not, and it is no decision.
    let reported = rig.ok(
        Some("rook"),
        &["seat", "fault", "the build cannot reach the mirror host"],
    );
    assert!(reported.contains("setup fault recorded"), "{reported}");
    assert!(
        !rig.ris(None, &["seat", "fault", "x"]).status.success(),
        "an agent's only"
    );
    assert_eq!(
        rig.status()["faults"],
        serde_json::json!(["the build cannot reach the mirror host"])
    );
    let text = rig.ok(None, &["status"]);
    assert!(
        text.contains("setup fault: the build cannot reach the mirror host"),
        "{text}"
    );
    let inbox: serde_json::Value =
        serde_json::from_str(&rig.ok(None, &["--json", "inbox"])).unwrap();
    assert_eq!(inbox["data"]["decisions"], serde_json::json!([]));

    // The next run clears it.
    rig.ok(None, &["up"]);
    rig.stopped();
    assert_eq!(rig.status()["faults"], serde_json::json!([]));

    // Turned off, a project whose build needs none of it is not held.
    rig.config("preflight = false");
    rig.ok(None, &["up"]);
}
