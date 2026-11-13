//! `ris up`, `ris down` and `ris logs`: the harness declared in
//! .osiris/config.toml is brought up by doing only the difference, as
//! docker compose up -d does, brought down without losing work, and
//! each seat's output read back. A fake agent program on PATH stands in
//! for the agent tool.

mod common;

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

    // Followed with --json, every line out is an envelope: JSON lines,
    // with no text or trailing document mixed in.
    let out = rig.ok(None, &["--json", "logs", "rook", "--follow"]);
    assert!(!out.trim().is_empty(), "nothing followed");
    for line in out.lines() {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(v["ok"], true, "{line}");
        assert_eq!(v["data"]["seat"], "rook", "{line}");
        assert!(v["data"]["line"].is_string(), "{line}");
    }
    assert!(out.contains("hello from rook"), "{out}");

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

/// The one rebase `ris up` does: a seat's sandbox refuses writes to a
/// rules file, so a branch with a commit of its own behind a change to
/// one cannot be rebased by its seat. Up rebases it, keeping the commit,
/// once the seat's agent has stopped; while it runs, it is left alone
/// and up says why.
#[test]
fn up_rebases_a_seat_over_a_rules_file_change() {
    let rig = Rig::new("up-rules", WAITING_AGENT);
    // The rebase makes a commit, so it needs an identity.
    rig.git(&["config", "user.name", "ris"]);
    rig.git(&["config", "user.email", "ris@localhost"]);
    rig.ok(None, &["up", "rook"]);
    let wt = rig.repo.with_file_name("repo-rook");
    fs::write(wt.join("mine"), "work\n").unwrap();
    rig.git_in(&wt, &["add", "mine"]);
    rig.git_in(&wt, &["commit", "-q", "-m", "mine"]);
    let own = rig.git_in(&wt, &["rev-parse", "HEAD"]);

    // The phase branch gains a commit that adds a rules file, made in a
    // worktree of its own that is then removed.
    let side = rig.repo.with_file_name("repo-side");
    rig.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        side.to_str().unwrap(),
        "phase/P-1",
    ]);
    fs::write(side.join("AGENTS.md"), "rules\n").unwrap();
    rig.git_in(&side, &["add", "AGENTS.md"]);
    rig.git_in(&side, &["commit", "-q", "-m", "rules"]);
    let tip = rig.git_in(&side, &["rev-parse", "HEAD"]);
    rig.git(&["update-ref", "refs/heads/phase/P-1", &tip]);
    rig.git(&["worktree", "remove", "--force", side.to_str().unwrap()]);

    // Its agent is running: left alone.
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: 1 behind phase/P-1, which changed AGENTS.md"),
        "{out}"
    );
    assert!(out.contains("left alone: its agent is running"), "{out}");
    assert_eq!(rig.git_in(&wt, &["rev-parse", "HEAD"]), own);

    // Stopped: rebased, with its own commit on top of the phase branch.
    rig.stop_all();
    rig.until("rook stopped", |r| r.events("run_stopped").len() == 1);
    let out = rig.ok(None, &["up", "rook"]);
    assert!(
        out.contains("rook: rebased onto phase/P-1 over its change to AGENTS.md"),
        "{out}"
    );
    assert_eq!(rig.git_in(&wt, &["rev-parse", "HEAD^"]), tip);
    assert_eq!(rig.git_in(&wt, &["log", "-1", "--format=%s"]), "mine");
    assert_eq!(head_branch(&wt), "rook");
    assert_eq!(fs::read_to_string(wt.join("AGENTS.md")).unwrap(), "rules\n");
    assert_eq!(fs::read_to_string(wt.join("mine")).unwrap(), "work\n");
}

/// A seat one commit ahead whose commit conflicts with the phase branch,
/// which has also changed a rules file: neither up nor the seat can
/// finish the rebase alone. Up keeps the commit on a branch named for
/// the seat, moves the worktree to the phase branch (writing the rules
/// file the seat's sandbox refuses), and tells the seat in its inbox to
/// cherry-pick the commit back and resolve the conflict itself.
#[test]
fn up_hands_a_conflicting_rebase_over_a_rules_file_change_back_to_the_seat() {
    let rig = Rig::new("up-rules-conflict", WAITING_AGENT);
    rig.git(&["config", "user.name", "ris"]);
    rig.git(&["config", "user.email", "ris@localhost"]);
    rig.ok(None, &["up", "rook"]);
    rig.ok(Some("heron"), &["item", "add", "x"]);
    rig.ok(Some("rook"), &["claim", "I-1"]);
    let wt = rig.repo.with_file_name("repo-rook");
    fs::write(wt.join("shared"), "rook's\n").unwrap();
    rig.git_in(&wt, &["add", "shared"]);
    rig.git_in(&wt, &["commit", "-q", "-m", "mine"]);
    let own = rig.git_in(&wt, &["rev-parse", "HEAD"]);
    let fork = rig.git_in(&wt, &["rev-parse", "HEAD^"]);

    // The phase branch adds a rules file and the same file, differently.
    let side = rig.repo.with_file_name("repo-side");
    rig.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        side.to_str().unwrap(),
        "phase/P-1",
    ]);
    fs::write(side.join("AGENTS.md"), "rules\n").unwrap();
    fs::write(side.join("shared"), "the phase's\n").unwrap();
    rig.git_in(&side, &["add", "AGENTS.md", "shared"]);
    rig.git_in(&side, &["commit", "-q", "-m", "rules"]);
    let tip = rig.git_in(&side, &["rev-parse", "HEAD"]);
    rig.git(&["update-ref", "refs/heads/phase/P-1", &tip]);
    rig.git(&["worktree", "remove", "--force", side.to_str().unwrap()]);

    rig.stop_all();
    rig.until("rook stopped", |r| r.events("run_stopped").len() == 1);
    let out = rig.ok(None, &["up", "rook"]);
    let kept = format!("rook-kept-{}", &own[..12]);
    assert!(
        out.contains(&format!(
            "rook: moved to phase/P-1; its commits conflict with it (in shared) and are kept on {kept}"
        )),
        "{out}"
    );
    assert!(out.contains("told in its inbox"), "{out}");

    // The worktree is on the phase branch, still on the seat's branch,
    // and the commit is kept.
    assert_eq!(rig.git_in(&wt, &["rev-parse", "HEAD"]), tip);
    assert_eq!(head_branch(&wt), "rook");
    assert_eq!(rig.git_in(&wt, &["rev-parse", &kept]), own);
    assert_eq!(fs::read_to_string(wt.join("AGENTS.md")).unwrap(), "rules\n");

    // The seat is told, about its item, the cherry-pick that brings the
    // commit back.
    let inbox = rig.ok(Some("rook"), &["inbox"]);
    let range = format!("{}..{kept}", &fork[..12]);
    assert!(
        inbox.contains(&format!("`git cherry-pick {range}`")),
        "{inbox}"
    );
    assert!(inbox.contains("about I-1"), "{inbox}");
    assert!(inbox.contains("`ris merge I-1`"), "{inbox}");

    // What it is told works: the cherry-pick stops on the conflict in
    // the seat's own file, not in a rules file.
    let pick = Command::new("git")
        .current_dir(&wt)
        .args(["cherry-pick", &range])
        .output()
        .unwrap();
    assert!(!pick.status.success());
    let unmerged = rig.git_in(&wt, &["diff", "--name-only", "--diff-filter=U"]);
    assert_eq!(unmerged, "shared");
}

/// A `ris seat start` killed outright never records its stop. Up sees
/// its process is gone, clears the run and starts the seat again,
/// instead of counting it as running for good.
#[test]
fn up_clears_a_run_whose_process_is_gone_and_starts_it_again() {
    let rig = Rig::new("up-gone", WAITING_AGENT);
    let rook_runs = |r: &Rig, event: &str| {
        r.events(event)
            .iter()
            .filter(|e| e["structure"]["seat"] == "rook")
            .count()
    };
    rig.ok(None, &["up"]);
    let lock = rig.repo.join(".osiris/runs/rook.lock");
    rig.until("rook's run names its process", |_| {
        fs::read_to_string(&lock).is_ok_and(|s| !s.trim().is_empty())
    });
    let pid = fs::read_to_string(&lock).unwrap().trim().to_owned();
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid])
            .status()
            .unwrap()
            .success()
    );
    // Its agent is left behind; let it go too, then clear the way for
    // the next one.
    let stop = rig.repo.join(".osiris/stop-rook");
    fs::write(&stop, "").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    fs::remove_file(&stop).unwrap();
    assert_eq!(
        rook_runs(&rig, "run_stopped"),
        0,
        "the kill recorded nothing"
    );

    let out = rig.ok(None, &["up"]);
    assert!(
        out.contains("rook: its last run ended without being recorded"),
        "{out}"
    );
    assert!(out.contains("rook: started"), "{out}");
    assert_eq!(rook_runs(&rig, "run_stopped"), 1);
    assert_eq!(rook_runs(&rig, "run_started"), 2);
    // The live ones are left as they are.
    assert!(!out.contains("heron:"), "{out}");
}

/// The summary's opening line names `ris up` only when no seat is
/// running, and a run whose process is gone does not count as one,
/// though the log still has it as running until `ris up` clears it.
#[test]
fn summary_says_when_no_seat_is_running() {
    let rig = Rig::new("up-summary", WAITING_AGENT);
    rig.ok(None, &["item", "add", "something to do"]);
    let state = |r: &Rig| -> Value {
        let v: Value = serde_json::from_slice(&r.ris(None, &["--json", "summary"]).stdout).unwrap();
        v["data"]["phase_state"].clone()
    };
    let none = "no seat is running: start them with `ris up`";
    let s = state(&rig);
    assert!(s["line"].as_str().unwrap().ends_with(none), "{s}");

    rig.ok(None, &["up", "rook"]);
    rig.until("rook running", |r| {
        state(r)["running"] == serde_json::json!(["rook"])
    });
    let s = state(&rig);
    assert!(!s["line"].as_str().unwrap().contains(none), "{s}");
    let text = rig.ok(None, &["summary"]);
    assert!(!text.lines().next().unwrap().contains(none), "{text}");

    // Its `ris seat start` killed outright: nothing records the stop.
    let lock = rig.repo.join(".osiris/runs/rook.lock");
    rig.until("rook's run names its process", |_| {
        fs::read_to_string(&lock).is_ok_and(|s| !s.trim().is_empty())
    });
    let pid = fs::read_to_string(&lock).unwrap().trim().to_owned();
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid])
            .status()
            .unwrap()
            .success()
    );
    fs::write(rig.repo.join(".osiris/stop-rook"), "").unwrap();
    rig.until("the gone run is not counted", |r| {
        state(r)["running"] == serde_json::json!([])
    });
    let s = state(&rig);
    assert!(s["line"].as_str().unwrap().ends_with(none), "{s}");
    assert_eq!(rig.events("run_stopped").len(), 0, "nothing was recorded");
}

/// A start that fails is reported as failing, with why, not as started.
#[test]
fn up_reports_a_start_that_fails() {
    let rig = Rig::new("up-fails", WAITING_AGENT);
    let config = rig.repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(&config, c.replace("fake-agent", "no-such-agent-tool")).unwrap();
    let out = rig.ok(None, &["up", "wren"]);
    assert!(
        out.contains("wren: failed to start: could not run `no-such-agent-tool`"),
        "{out}"
    );
    assert!(!out.contains("wren: started"), "{out}");
}

/// A seat marked `start = false` is attended: a person runs it. Up gives
/// it a worktree and nothing more; down does not pause it; a pause the
/// person set on it is not undone by up.
#[test]
fn up_and_down_leave_an_attended_seat_alone() {
    let rig = Rig::new("up-attended", WAITING_AGENT);
    let config = rig.repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace(
            "[agents.rook]\nrole = \"builder\"\n",
            "[agents.rook]\nrole = \"builder\"\nstart = false\n",
        ),
    )
    .unwrap();
    let seats_of = |r: &Rig, event: &str| -> Vec<String> {
        r.events(event)
            .iter()
            .map(|e| e["structure"]["seat"].as_str().unwrap().to_owned())
            .collect()
    };
    let rook = "rook".to_owned();

    let out = rig.ok(None, &["up"]);
    assert!(out.contains("rook: worktree at ../repo-rook"), "{out}");
    assert!(!out.contains("rook: started"), "{out}");
    assert!(
        out.contains("attended (start = false), left alone: rook"),
        "{out}"
    );
    rig.until("heron and wren started", |r| {
        seats_of(r, "run_started").len() == 2
    });
    assert!(!seats_of(&rig, "run_started").contains(&rook));

    // Down pauses the unattended seats only.
    let out = rig.ok(None, &["down"]);
    assert!(
        out.contains("heron: paused") && out.contains("wren: paused"),
        "{out}"
    );
    assert!(out.contains("left alone: rook"), "{out}");
    assert!(!seats_of(&rig, "seat_paused").contains(&rook));

    // The person pauses rook by hand; up resumes the others, not rook,
    // and still starts nothing for it.
    rig.ok(None, &["seat", "pause", "rook"]);
    let out = rig.ok(None, &["up"]);
    assert!(
        out.contains("heron: resumed") && !out.contains("rook: resumed"),
        "{out}"
    );
    assert!(!seats_of(&rig, "seat_resumed").contains(&rook));
    assert!(!seats_of(&rig, "run_started").contains(&rook));
}

/// Watching the seats: `ris logs` with no seat prints every seat's
/// output with its seat in front, and followed it streams them all
/// until none is running; `ris status` shows what each running seat
/// last did and how long ago, and marks one quiet for longer than
/// `[runner] quiet_after_seconds` as possibly stuck.
#[test]
fn every_seat_is_watched_in_one_stream_and_a_quiet_one_is_marked() {
    let rig = Rig::new("watch-seats", WAITING_AGENT);
    let prefixed = [
        "heron | hello from heron",
        "rook  | hello from rook",
        "wren  | hello from wren",
    ];
    rig.ok(None, &["up"]);
    rig.until("every seat has spoken", |r| {
        let out = r.ok(None, &["logs"]);
        prefixed.iter().all(|p| out.lines().any(|l| l == *p))
    });

    // Status: each running seat's last activity, not stuck by default.
    let status = |r: &Rig| -> Value {
        serde_json::from_slice(&r.ris(None, &["--json", "status"]).stdout).unwrap()
    };
    let seat = |v: &Value, name: &str| -> Value {
        v["data"]["seats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["seat"] == name)
            .unwrap()
            .clone()
    };
    let rook = seat(&status(&rig), "rook");
    assert_eq!(rook["activity"]["last"], "hello from rook", "{rook}");
    assert_eq!(rook["activity"]["stuck"], false, "{rook}");
    assert!(rook["activity"]["quiet_seconds"].is_u64(), "{rook}");
    let text = rig.ok(None, &["status"]);
    assert!(text.contains("ago: hello from rook"), "{text}");
    assert!(!text.contains("possibly stuck"), "{text}");

    // With no quiet time allowed, a seat that has said nothing since is
    // marked.
    let config = rig.repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace("[runner]\n", "[runner]\nquiet_after_seconds = 0\n"),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let rook = seat(&status(&rig), "rook");
    assert_eq!(rook["activity"]["stuck"], true, "{rook}");
    let text = rig.ok(None, &["status"]);
    assert!(text.contains("possibly stuck: quiet"), "{text}");

    // Followed with no seat: one stream of every seat, which ends once
    // no seat is running. With --json, one envelope per line.
    let path = format!(
        "{}:{}",
        rig.bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let follow = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ris"))
            .args(args)
            .current_dir(&rig.repo)
            .env("RIS_AGENT", "person")
            .env_remove("RIS_ROOT")
            .env("NO_COLOR", "1")
            .env("PATH", &path)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let text = follow(&["logs", "-f"]);
    let json = follow(&["--json", "logs", "--follow"]);
    std::thread::sleep(Duration::from_millis(300));
    rig.stop_all();
    let text = String::from_utf8(text.wait_with_output().unwrap().stdout).unwrap();
    for p in prefixed {
        assert!(text.lines().any(|l| l == p), "{p} in:\n{text}");
    }
    let json = String::from_utf8(json.wait_with_output().unwrap().stdout).unwrap();
    // Each followed line is an envelope of the committed schema.
    common::validate_output(&["logs", "--follow"], &json);
    let seats: Vec<String> = json
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}"));
            assert_eq!(v["ok"], true, "{l}");
            assert!(v["data"]["line"].is_string(), "{l}");
            v["data"]["seat"].as_str().unwrap().to_owned()
        })
        .collect();
    for s in ["heron", "rook", "wren"] {
        assert!(seats.iter().any(|x| x == s), "{s} in:\n{json}");
    }

    // Nothing running: no activity is shown.
    assert!(
        ["heron", "rook", "wren"]
            .iter()
            .all(|s| seat(&status(&rig), s)["activity"].is_null())
    );
}
