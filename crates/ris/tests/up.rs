//! `ris up`: the harness declared in .osiris/config.toml is brought up
//! by doing only the difference, as docker compose up -d does. A fake
//! agent program on PATH stands in for the agent tool.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;

struct Rig {
    repo: PathBuf,
    bin: PathBuf,
}

impl Rig {
    fn new(name: &str, agent_script: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ris-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let bin = base.join("bin");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("fake-agent");
        fs::write(&fake, agent_script).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let rig = Self { repo, bin };
        rig.git(&["init", "-q", "-b", "main"]);
        fs::write(rig.repo.join("README"), "base\n").unwrap();
        rig.git(&["add", "-A"]);
        rig.git(&["commit", "-q", "-m", "base"]);
        rig.ok(None, &["init", "--local"]);
        fs::write(
            rig.repo.join(".osiris/checks.toml"),
            "[checks.ok]\ncommand = \"true\"\n",
        )
        .unwrap();
        fs::write(
            rig.repo.join(".osiris/config.toml"),
            "denylist = [\"example client\"]\n[runner]\nprogram = \"fake-agent\"\n\
             [agents.heron]\nrole = \"lead\"\n[agents.rook]\nrole = \"builder\"\n\
             [agents.wren]\nrole = \"auditor\"\n",
        )
        .unwrap();
        rig.ok(None, &["phase", "open", "up"]);
        rig
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

    fn events(&self, name: &str) -> Vec<Value> {
        self.ok(None, &["log"])
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|e| e["structure"]["event"] == name)
            .collect()
    }

    /// Wait until `f` holds, or fail after a few seconds.
    fn until(&self, what: &str, f: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !f(self) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The fake agent prints who it is and waits until a file appears, so
/// the test decides when each agent stops.
const WAITING_AGENT: &str = "#!/bin/sh\nprintf 'hello from %s\\n' \"$RIS_AGENT\"\n\
    while [ ! -f \"$RIS_ROOT/.osiris/stop-$RIS_AGENT\" ]; do sleep 0.05; done\nexit 0\n";

#[test]
fn up_does_only_the_difference() {
    let rig = Rig::new("up", WAITING_AGENT);
    let running = |r: &Rig| -> Vec<String> {
        let v: Value = serde_json::from_slice(&r.ris(None, &["--json", "status"]).stdout).unwrap();
        v["data"]["seats"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["running"] == true)
            .map(|s| s["seat"].as_str().unwrap().to_owned())
            .collect()
    };

    // An agent cannot bring the harness up.
    assert_eq!(rig.ris(Some("heron"), &["up"]).status.code(), Some(2));

    // First up: rook gets its worktree; all three start.
    let out = rig.ok(None, &["up"]);
    assert!(out.contains("rook: worktree at ../repo-rook"), "{out}");
    for seat in ["heron", "rook", "wren"] {
        assert!(out.contains(&format!("{seat}: started")), "{out}");
    }
    rig.until("all three running", |r| running(r).len() == 3);

    // Again: nothing to do.
    let out = rig.ok(None, &["up"]);
    assert_eq!(
        out.trim(),
        "nothing to do: the harness matches .osiris/config.toml"
    );
    assert_eq!(rig.events("run_started").len(), 3, "nothing started twice");

    // Each agent stops when it is done; up starts only the named seat
    // again.
    for seat in ["heron", "rook", "wren"] {
        fs::write(rig.repo.join(format!(".osiris/stop-{seat}")), "").unwrap();
    }
    rig.until("all stopped", |r| running(r).is_empty());
    assert_eq!(rig.events("run_stopped").len(), 3);
    fs::remove_file(rig.repo.join(".osiris/stop-rook")).unwrap();
    let out = rig.ok(None, &["up", "rook"]);
    assert!(out.contains("rook: started"), "{out}");
    assert!(!out.contains("heron"), "{out}");
    rig.until("rook running", |r| running(r) == ["rook"]);
    fs::write(rig.repo.join(".osiris/stop-rook"), "").unwrap();
    rig.until("rook stopped", |r| running(r).is_empty());

    // A seat in the log but removed from the file is reported, untouched.
    rig.ok(Some("heron"), &["item", "add", "x"]);
    rig.ok(Some("heron"), &["claim", "I-1"]);
    rig.ok(
        Some("heron"),
        &[
            "decision",
            "raise",
            "--about",
            "I-1",
            "--question",
            "q",
            "--option",
            "a",
            "--option",
            "b",
            "--recommend",
            "0",
            "--rationale",
            "r",
        ],
    );
    let config = rig.repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(&config, c.replace("[agents.heron]\nrole = \"lead\"\n", "")).unwrap();
    let out = rig.ok(None, &["up", "wren"]);
    assert!(
        out.contains("in the log but not in the file, left alone: heron"),
        "{out}"
    );
    fs::write(rig.repo.join(".osiris/stop-wren"), "").unwrap();
    rig.until("wren stopped", |r| running(r).is_empty());
}
