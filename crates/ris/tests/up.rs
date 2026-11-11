//! `ris up`, `ris down` and `ris logs`: the harness declared in
//! .osiris/config.toml is brought up by doing only the difference, as
//! docker compose up -d does, brought down without losing work, and
//! each seat's output read back. A fake agent program on PATH stands in
//! for the agent tool.

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
            "denylist = [\"example client\"]\ndocs_item = false\n[runner]\nprogram = \"fake-agent\"\n\
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

    /// Write every seat's stop file, so its fake agent exits.
    fn stop_all(&self) {
        for seat in ["heron", "rook", "wren"] {
            let _ = fs::write(self.repo.join(format!(".osiris/stop-{seat}")), "");
        }
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

/// A failing test must not leave agents running in the background:
/// they would outlive it and slow every later run.
impl Drop for Rig {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The fake agent prints who it is and waits until a file appears, so
/// the test decides when each agent stops.
const WAITING_AGENT: &str = "#!/bin/sh\nprintf 'hello from %s\\n' \"$RIS_AGENT\"\n\
    while [ ! -f \"$RIS_ROOT/.osiris/stop-$RIS_AGENT\" ]; do sleep 0.05; done\nexit 0\n";

/// `worktrees` in the configuration places a builder's worktree; one
/// inside the main checkout leaves that checkout clean.
#[test]
fn up_puts_a_worktree_where_the_configuration_says() {
    let rig = Rig::new("up-placed", WAITING_AGENT);
    let config = rig.repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("worktrees = \".seats/{{seat}}\"\n{c}")).unwrap();

    let out = rig.ok(None, &["up", "rook"]);
    assert!(out.contains("rook: worktree at .seats/rook"), "{out}");
    let wt = rig.repo.join(".seats/rook");
    assert!(wt.join("README").exists(), "a checkout of the repository");
    assert!(
        !rig.repo.with_file_name("repo-rook").exists(),
        "not at the default place"
    );

    let v: Value = serde_json::from_slice(&rig.ris(None, &["--json", "status"]).stdout).unwrap();
    let rook = v["data"]["seats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["seat"] == "rook")
        .unwrap();
    assert_eq!(rook["worktree"], ".seats/rook", "{rook}");

    let porcelain = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&rig.repo)
        .output()
        .unwrap();
    assert!(
        porcelain.stdout.is_empty(),
        "the main checkout stays clean: {}",
        String::from_utf8_lossy(&porcelain.stdout)
    );

    // A template every seat would share is refused before anything runs.
    fs::write(&config, format!("worktrees = \"../shared\"\n{c}")).unwrap();
    let out = rig.ris(None, &["status"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("must contain {seat}"));
}

#[test]
fn up_does_only_the_difference_and_down_stops_without_losing_work() {
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

    // First up: each writing seat, the lead included, gets its own
    // worktree (D-43); the auditor reads the main checkout. All three
    // start.
    let out = rig.ok(None, &["up"]);
    assert!(out.contains("rook: worktree at ../repo-rook"), "{out}");
    assert!(out.contains("heron: worktree at ../repo-heron"), "{out}");
    assert!(!out.contains("wren: worktree"), "{out}");
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

    // Each seat's output reads back as text.
    rig.until("rook's output", |r| {
        r.ok(None, &["logs", "rook"]).contains("hello from rook")
    });

    // Down: every seat is paused and lets its agent finish.
    let out = rig.ok(None, &["down"]);
    assert!(out.contains("rook: paused"), "{out}");
    for seat in ["heron", "rook", "wren"] {
        fs::write(rig.repo.join(format!(".osiris/stop-{seat}")), "").unwrap();
    }
    rig.until("all stopped", |r| running(r).is_empty());
    assert_eq!(rig.events("run_stopped").len(), 3);
    assert!(
        rig.repo.with_file_name("repo-rook").exists(),
        "worktrees are kept"
    );
    let out = rig.ok(None, &["down"]);
    assert!(
        out.starts_with("nothing to do"),
        "down is idempotent: {out}"
    );

    // Up after down resumes and starts again; only the named seat.
    for seat in ["heron", "rook", "wren"] {
        fs::remove_file(rig.repo.join(format!(".osiris/stop-{seat}"))).unwrap();
    }
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: resumed") && out.contains("rook: started"),
        "{out}"
    );
    assert!(!out.contains("heron"), "{out}");
    rig.until("rook running", |r| running(r) == ["rook"]);
    fs::write(rig.repo.join(".osiris/stop-rook"), "").unwrap();
    rig.until("rook stopped", |r| running(r).is_empty());

    // A seat in the log but removed from the file is reported, untouched.
    rig.ok(None, &["seat", "resume", "heron"]);
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

fn head_branch(dir: &std::path::Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A main checkout left on the phase branch by the layout before D-43
/// goes back to the main line at `ris up`, so seats' merges can move the
/// phase branch by a ref update; one with uncommitted changes is left
/// where it is, and up says so.
#[test]
fn up_takes_the_main_checkout_off_the_phase_branch() {
    let rig = Rig::new("up-layout", WAITING_AGENT);
    rig.git(&["switch", "-q", "phase/P-1"]);
    fs::write(rig.repo.join("README"), "changed\n").unwrap();
    let out = rig.ok(None, &["up", "wren"]);
    assert!(out.contains("left on phase/P-1"), "{out}");
    assert_eq!(head_branch(&rig.repo), "phase/P-1");
    rig.stop_all();

    rig.git(&["checkout", "-q", "--", "README"]);
    let out = rig.ok(None, &["up", "wren"]);
    assert!(out.contains("main checkout: on main"), "{out}");
    assert_eq!(head_branch(&rig.repo), "main");
}

impl Rig {
    /// Run git in `dir` and return its trimmed output.
    fn git_in(&self, dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {}", args.join(" "));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    /// Move the phase branch on by one empty commit, as another seat's
    /// merge would. The new tip.
    fn advance_phase(&self) -> String {
        let next = self.git_in(
            &self.repo,
            &[
                "commit-tree",
                "phase/P-1^{tree}",
                "-p",
                "phase/P-1",
                "-m",
                "merged elsewhere",
            ],
        );
        self.git_in(&self.repo, &["update-ref", "refs/heads/phase/P-1", &next]);
        next
    }
}

/// A builder's worktree behind the phase branch is fast-forwarded by
/// `ris up` only when that loses nothing: no claim, no commits of its
/// own, nothing uncommitted. Otherwise it is left where it is and up
/// says why.
#[test]
fn up_brings_an_idle_worktree_up_to_the_phase_branch() {
    let rig = Rig::new("up-stale", WAITING_AGENT);
    rig.ok(None, &["up", "rook"]);
    let wt = rig.repo.with_file_name("repo-rook");
    let head = |r: &Rig| r.git_in(&wt, &["rev-parse", "HEAD"]);

    // Idle and clean: brought up, still on its own branch.
    let tip = rig.advance_phase();
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: brought up to phase/P-1 (1 commits)"),
        "{out}"
    );
    assert_eq!(head(&rig), tip);
    assert_eq!(head_branch(&wt), "rook");

    // Up to date: nothing said about it.
    let out = rig.ok(None, &["up", "rook"]);
    assert!(!out.contains("phase/P-1"), "{out}");

    // Uncommitted changes: left alone.
    let tip = rig.advance_phase();
    fs::write(wt.join("scratch"), "work\n").unwrap();
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: 1 behind phase/P-1, left alone: its worktree has uncommitted changes"),
        "{out}"
    );
    assert_ne!(head(&rig), tip);
    fs::remove_file(wt.join("scratch")).unwrap();

    // A claimed item: left alone.
    rig.ok(Some("heron"), &["item", "add", "x"]);
    rig.ok(Some("rook"), &["claim", "I-1"]);
    let out = rig.ok(None, &["up", "rook"]);
    assert!(out.contains("left alone: it holds I-1"), "{out}");
    assert_ne!(head(&rig), tip);
}

/// A worktree with a commit of its own is never moved by `ris up`: no
/// rebase, no reset.
#[test]
fn up_leaves_a_worktree_with_its_own_commits_alone() {
    let rig = Rig::new("up-own", WAITING_AGENT);
    rig.ok(None, &["up", "rook"]);
    let wt = rig.repo.with_file_name("repo-rook");
    fs::write(wt.join("mine"), "work\n").unwrap();
    rig.git_in(&wt, &["add", "mine"]);
    rig.git_in(&wt, &["commit", "-q", "-m", "mine"]);
    let own = rig.git_in(&wt, &["rev-parse", "HEAD"]);
    rig.advance_phase();
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: 1 behind phase/P-1, left alone: its branch holds commits of its own"),
        "{out}"
    );
    assert_eq!(rig.git_in(&wt, &["rev-parse", "HEAD"]), own);
}
