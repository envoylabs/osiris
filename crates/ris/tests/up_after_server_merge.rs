//! `ris up` after a phase closed on a git server's squash or rebase:
//! a writing seat's branch still holds the closed phase's commits,
//! which are ancestors of nothing on the main line, so no fast-forward
//! reaches the next phase's branch. Where every commit of its own is a
//! merged item of a phase whose work the log records on the main line,
//! the seat holds nothing that can be lost and is moved; a commit that
//! no such phase carries still leaves it alone. A fake agent program on
//! PATH stands in for the agent tool.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// The fake agent waits until a file appears, so the test decides when
/// it stops.
const WAITING_AGENT: &str = "#!/bin/sh\n\
    while [ ! -f \"$RIS_ROOT/.osiris/stop-$RIS_AGENT\" ]; do sleep 0.05; done\nexit 0\n";

struct Rig {
    base: PathBuf,
    repo: PathBuf,
    bin: PathBuf,
    /// The builder's worktree.
    rook: PathBuf,
}

impl Rig {
    /// A project that leaves the closing merge to the git server, with
    /// a phase open and a builder's worktree in it.
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ris-up-after-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let bin = base.join("bin");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("fake-agent");
        fs::write(&fake, WAITING_AGENT).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let rig = Self {
            rook: base.join("repo-rook"),
            base,
            repo,
            bin,
        };
        rig.git(&rig.repo, &["init", "-q", "-b", "main"]);
        // A seat's merge rebases, which makes a commit: it needs an
        // identity, set here so no machine's own is read.
        rig.git(&rig.repo, &["config", "user.name", "ris"]);
        rig.git(&rig.repo, &["config", "user.email", "ris@localhost"]);
        fs::write(rig.repo.join("README"), "base\n").unwrap();
        rig.git(&rig.repo, &["add", "-A"]);
        rig.git(&rig.repo, &["commit", "-q", "-m", "base"]);
        rig.ok(&rig.repo, "person", &["init", "--local"]);
        fs::write(
            rig.repo.join(".osiris/checks.toml"),
            "[checks.ok]\ncommand = \"true\"\n",
        )
        .unwrap();
        fs::write(
            rig.repo.join(".osiris/config.toml"),
            "denylist = [\"example client\"]\ndocs_item = false\nmerge_on_close = false\n\
             [runner]\nprogram = \"fake-agent\"\n\
             [agents.heron]\nrole = \"lead\"\n[agents.rook]\nrole = \"builder\"\n",
        )
        .unwrap();
        rig.ok(&rig.repo, "person", &["phase", "open", "the first"]);
        rig.ok(&rig.repo, "person", &["up", "rook"]);
        rig
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
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

    fn ris(&self, dir: &Path, agent: &str, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_ris"))
            .args(args)
            .current_dir(dir)
            .env("RIS_AGENT", agent)
            .env_remove("RIS_ROOT")
            .env("NO_COLOR", "1")
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn ok(&self, dir: &Path, agent: &str, args: &[&str]) -> String {
        let out = self.ris(dir, agent, args);
        assert!(
            out.status.success(),
            "ris {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// The builder works one item of the open phase in its worktree and
    /// merges it. The merged commit.
    fn rook_merges(&self, item: &str, file: &str) -> String {
        self.ok(
            &self.repo,
            "person",
            &["item", "add", file, "--check", "ok"],
        );
        self.ok(&self.rook, "rook", &["claim", item]);
        fs::write(self.rook.join(file), "work\n").unwrap();
        self.ok(&self.rook, "rook", &["done", item]);
        self.git(&self.rook, &["add", "-A"]);
        self.git(
            &self.rook,
            &[
                "commit",
                "-q",
                "-m",
                &format!("feat: {file}"),
                "-m",
                &format!("Item: {item}\nAgent: rook"),
            ],
        );
        self.ok(&self.rook, "rook", &["merge", item]);
        self.git(&self.rook, &["rev-parse", "HEAD"])
    }

    /// Stop the builder's agent and wait until the log says it stopped.
    fn stop_rook(&self) {
        fs::write(self.repo.join(".osiris/stop-rook"), "").unwrap();
        let start = Instant::now();
        while !self
            .ok(&self.repo, "person", &["log"])
            .contains("\"run_stopped\"")
        {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out: rook's agent stopped"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Someone else's commit on the main line, so the server's merge is
    /// not a fast-forward.
    fn main_moves_on(&self) {
        fs::write(self.repo.join("theirs"), "theirs\n").unwrap();
        self.git(&self.repo, &["add", "-A"]);
        self.git(&self.repo, &["commit", "-q", "-m", "feat: theirs"]);
    }

    /// Close the first phase, whose work is on the main line by now,
    /// and open the next.
    fn next_phase(&self) {
        let closed = self.ok(&self.repo, "person", &["phase", "close", "--unreviewed"]);
        assert!(closed.contains("closing merge: made elsewhere"), "{closed}");
        self.ok(&self.repo, "person", &["phase", "open", "the next"]);
    }

    fn head(&self) -> String {
        self.git(&self.rook, &["rev-parse", "HEAD"])
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // No agent outlives its test.
        for seat in ["heron", "rook"] {
            let _ = fs::write(self.repo.join(format!(".osiris/stop-{seat}")), "");
        }
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The seat moved: on its own branch still, at the next phase's
/// branch, with the main line's files and its own work, as merged.
fn moved(rig: &Rig, out: &str, own: &str) {
    assert!(
        out.contains(
            "rook: moved to phase/P-2: the 1 commit(s) of its own are merged items of P-1, \
             whose work is on main, so nothing is lost"
        ),
        "{out}"
    );
    assert_ne!(rig.head(), own);
    assert_eq!(rig.head(), rig.git(&rig.repo, &["rev-parse", "phase/P-2"]));
    assert_eq!(
        rig.git(&rig.rook, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "rook"
    );
    assert_eq!(fs::read_to_string(rig.rook.join("a")).unwrap(), "work\n");
    assert_eq!(
        fs::read_to_string(rig.rook.join("theirs")).unwrap(),
        "theirs\n"
    );
}

#[test]
fn a_seat_is_brought_to_the_next_phase_after_a_squash() {
    let rig = Rig::new("squash");
    let own = rig.rook_merges("I-1", "a");
    rig.stop_rook();
    // The server's squash: one commit on the main line with the
    // phase's whole change, and none of the phase's own commits.
    rig.main_moves_on();
    rig.git(&rig.repo, &["merge", "-q", "--squash", "phase/P-1"]);
    rig.git(&rig.repo, &["commit", "-q", "-m", "The first (#1)"]);
    rig.next_phase();
    assert!(
        !Command::new("git")
            .args(["merge-base", "--is-ancestor", &own, "phase/P-2"])
            .current_dir(&rig.repo)
            .status()
            .unwrap()
            .success(),
        "the seat's commit is no ancestor of the next phase's branch"
    );

    let out = rig.ok(&rig.repo, "person", &["up", "rook"]);
    moved(&rig, &out, &own);
}

#[test]
fn a_seat_is_brought_to_the_next_phase_after_a_rebase() {
    let rig = Rig::new("rebase");
    let own = rig.rook_merges("I-1", "a");
    rig.stop_rook();
    // The server's rebase: the phase's commit made again on a main
    // line that has moved, as another commit.
    rig.main_moves_on();
    rig.git(&rig.repo, &["cherry-pick", "phase/P-1"]);
    assert_ne!(rig.git(&rig.repo, &["rev-parse", "main"]), own);
    rig.next_phase();

    let out = rig.ok(&rig.repo, "person", &["up", "rook"]);
    moved(&rig, &out, &own);
}

#[test]
fn a_seat_with_a_commit_no_closed_phase_carries_is_left_alone() {
    let rig = Rig::new("own");
    rig.rook_merges("I-1", "a");
    rig.stop_rook();
    rig.main_moves_on();
    rig.git(&rig.repo, &["merge", "-q", "--squash", "phase/P-1"]);
    rig.git(&rig.repo, &["commit", "-q", "-m", "The first (#1)"]);
    rig.next_phase();
    // Work of its own on top, merged nowhere: moving the seat would
    // lose it.
    fs::write(rig.rook.join("mine"), "mine\n").unwrap();
    rig.git(&rig.rook, &["add", "-A"]);
    rig.git(&rig.rook, &["commit", "-q", "-m", "mine"]);
    let own = rig.head();

    let out = rig.ok(&rig.repo, "person", &["up", "rook"]);
    assert!(
        out.contains("behind phase/P-2, left alone: its branch holds commits of its own"),
        "{out}"
    );
    assert!(!out.contains("rook: moved"), "{out}");
    assert_eq!(rig.head(), own);
    assert_eq!(fs::read_to_string(rig.rook.join("mine")).unwrap(), "mine\n");
}
