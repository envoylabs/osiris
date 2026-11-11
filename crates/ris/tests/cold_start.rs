//! The cold-start drill a seat can run (D-45): a fresh seat is started
//! through the runner in a scratch project with an open phase, and its
//! run log, as the runner keeps it, is checked by `ris::drill`. A fake
//! agent tool plays a transcript, so the test shows the drill passes a
//! seat that picks up by the rules and names each way one can fail.
//! The real thing, on a real model, is the person's to judge.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
}

struct Scratch {
    base: PathBuf,
    repo: PathBuf,
    bin: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ris-cold-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let bin = base.join("bin");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&bin).unwrap();
        // The fake agent tool plays the transcript it is given, as the
        // agent tool's stream JSON on its output.
        let fake = bin.join("fake-agent");
        fs::write(&fake, "#!/bin/sh\ncat \"$RIS_ROOT/.osiris/transcript\"\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let s = Self { base, repo, bin };
        git(&s.repo, &["init", "-q", "-b", "main"]);
        fs::write(s.repo.join("README"), "base\n").unwrap();
        git(&s.repo, &["add", "-A"]);
        git(&s.repo, &["commit", "-q", "-m", "base"]);
        s.ok(&["init", "--local"]);
        fs::write(
            s.repo.join(".osiris/config.toml"),
            "denylist = [\"example client\"]\ndocs_item = false\n\
             [runner]\nprogram = \"fake-agent\"\n[agents.rook]\nrole = \"builder\"\n",
        )
        .unwrap();
        s.ok(&["phase", "open", "a scratch phase"]);
        s.ok(&["item", "add", "something to do"]);
        s.ok(&["worktree", "rook"]);
        s
    }

    fn ris(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_ris"))
            .args(args)
            .current_dir(&self.repo)
            .env("RIS_AGENT", "person")
            .env_remove("RIS_ROOT")
            .env("NO_COLOR", "1")
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) {
        let out = self.ris(args);
        assert!(
            out.status.success(),
            "ris {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Start the fresh seat on `transcript` and return the drill's
    /// faults from its run log.
    fn drill(&self, transcript: &[String]) -> Vec<String> {
        fs::write(
            self.repo.join(".osiris/transcript"),
            transcript.join("\n") + "\n",
        )
        .unwrap();
        self.ok(&["seat", "start", "rook", "--quiet"]);
        let runs: Vec<PathBuf> = fs::read_dir(self.repo.join(".osiris/runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        assert_eq!(runs.len(), 1, "one run, of a fresh seat: {runs:?}");
        ris::drill::cold_start(&fs::read_to_string(&runs[0]).unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn bash(c: &str) -> String {
    serde_json::json!({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "name": "Bash", "input": {"command": c}}
    ]}})
    .to_string()
}

fn read(path: &str) -> String {
    serde_json::json!({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "name": "Read", "input": {"file_path": path}}
    ]}})
    .to_string()
}

#[test]
fn a_fresh_seat_that_picks_up_by_the_rules_passes_the_drill() {
    let s = Scratch::new("pass");
    let faults = s.drill(&[
        r#"{"type":"system","subtype":"init"}"#.to_owned(),
        bash("ris inbox"),
        bash("ris ready"),
        bash("ris report"),
        bash("ris claim I-1"),
        read("README.md"),
        r#"{"type":"result","subtype":"success"}"#.to_owned(),
    ]);
    assert_eq!(faults, Vec::<String>::new());
}

#[test]
fn a_fresh_seat_that_reads_a_plan_first_or_acts_as_the_person_fails_the_drill() {
    let s = Scratch::new("fail");
    let faults = s.drill(&[
        read("PLAN.md"),
        bash("ris claim I-1"),
        bash("ris inbox"),
        bash("RIS_AGENT=person ris decide D-1 --choose 0 --rationale go"),
    ]);
    for want in [
        "used Read before running ris inbox, ready and report",
        "not inbox, ready and report",
        "read a document with Read before claiming anything",
        "set RIS_AGENT itself",
        "ran a person's command",
    ] {
        assert!(
            faults.iter().any(|f| f.contains(want)),
            "{want} in {faults:?}"
        );
    }
}
