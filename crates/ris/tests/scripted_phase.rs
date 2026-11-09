//! Drives the built `ris` binary through a scripted phase: two agents,
//! three items, one mechanical and one substantive decision, a message,
//! a merging and a close. Output is compared to a golden file with
//! hashes normalised. Set `UPDATE_GOLDEN=1` to rewrite the golden file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

struct Sandbox {
    dir: PathBuf,
    transcript: String,
}

impl Sandbox {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ris-cli-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let git = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(git.success());
        Self {
            dir,
            transcript: String::new(),
        }
    }

    /// Run `ris` as `agent` (or the person when `None`), recording the
    /// call and its output in the transcript.
    fn ris(&mut self, agent: Option<&str>, args: &[&str]) -> Run {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
        cmd.args(args)
            .current_dir(&self.dir)
            .env_remove("RIS_AGENT");
        if let Some(a) = agent {
            cmd.env("RIS_AGENT", a);
        }
        let out = cmd.output().unwrap();
        let run = Run {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        };
        self.transcript.push_str(&format!(
            "$ {}ris {}\n",
            agent.map(|a| format!("RIS_AGENT={a} ")).unwrap_or_default(),
            args.join(" ")
        ));
        if !run.stdout.is_empty() {
            self.transcript.push_str(&run.stdout);
        }
        if !run.stderr.is_empty() {
            self.transcript
                .push_str(&format!("[stderr] {}", run.stderr));
        }
        self.transcript
            .push_str(&format!("[exit {}]\n\n", run.status));
        run
    }

    fn ok(&mut self, agent: Option<&str>, args: &[&str]) -> Run {
        let r = self.ris(agent, args);
        assert_eq!(r.status, 0, "ris {} failed: {}", args.join(" "), r.stderr);
        r
    }

    fn refused(&mut self, agent: Option<&str>, args: &[&str]) -> Run {
        let r = self.ris(agent, args);
        assert_eq!(
            r.status,
            2,
            "ris {} should be refused: {}{}",
            args.join(" "),
            r.stdout,
            r.stderr
        );
        assert!(
            r.stderr.contains("refused:"),
            "stderr names the refusal: {}",
            r.stderr
        );
        assert!(
            r.stderr.contains("next:"),
            "stderr says what to do next: {}",
            r.stderr
        );
        r
    }
}

/// Make the transcript deterministic and clean: the sandbox path
/// becomes `<dir>`, the planted path canary `<canary>`, and 64-hex
/// hashes `<hash>` (salts are random, so hashes differ between runs).
fn normalise(s: &str, dir: &Path) -> String {
    let s = s.replace(&dir.display().to_string(), "<dir>");
    let s = s.replace(&path_canary(), "<canary>");
    hashes(&s)
}

/// An identifying path, assembled so the literal never sits in the
/// repository: the diff check would rightly refuse it.
fn path_canary() -> String {
    ["/Us", "ers/someone/src/x.rs"].concat()
}

fn hashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() == 64 {
            out.push_str("<hash>");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_hexdigit() && !c.is_ascii_uppercase() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/scripted_phase.txt")
}

#[test]
fn scripted_phase_matches_the_golden_transcript() {
    let mut sb = Sandbox::new();
    let heron = Some("heron");
    let wren = Some("wren");

    // Not a project yet.
    let r = sb.ris(None, &["ready"]);
    assert_eq!(r.status, 1);
    assert!(r.stderr.contains("ris init"));

    sb.ok(None, &["init"]);
    assert!(
        fs::read_to_string(sb.dir.join(".gitignore"))
            .unwrap()
            .contains(".osiris/")
    );
    let r = sb.ris(None, &["init"]);
    assert_eq!(r.status, 1, "a second init is an error");

    // Nothing without a phase.
    sb.refused(None, &["item", "add", "too early"]);
    sb.ok(None, &["phase", "open", "first slice"]);
    sb.refused(None, &["phase", "open", "second at once"]);

    sb.ok(
        None,
        &["item", "add", "the event envelope", "--check", "test"],
    );
    sb.ok(None, &["item", "add", "the fold", "--depends", "I-1"]);
    sb.ok(None, &["item", "add", "the cli"]);
    let r = sb.ris(None, &["item", "add", "bad check", "--check", "nope"]);
    assert_eq!(r.status, 1);

    // The scan at the command line.
    let canary = path_canary();
    let r = sb.refused(None, &["item", "add", &format!("see {canary}")]);
    assert!(r.stderr.contains("absolute path"));
    assert!(!r.stderr.contains(&canary[..12]), "the match is not echoed");

    sb.ok(None, &["ready"]);
    let r = sb.ris(None, &["claim", "I-1"]);
    assert_eq!(r.status, 1, "the person does not claim");
    sb.ok(heron, &["claim", "I-1"]);
    sb.refused(wren, &["claim", "I-1"]);
    sb.refused(wren, &["claim", "I-2"]); // depends on I-1
    sb.ok(heron, &["claim", "I-3"]);

    // A mechanical decision: taken at once by the raiser.
    sb.ok(
        heron,
        &[
            "decision",
            "raise",
            "--about",
            "I-1",
            "--question",
            "sha2 or blake3?",
            "--option",
            "sha2",
            "--option",
            "blake3",
            "--recommend",
            "0",
            "--rationale",
            "checkable with standard tools",
        ],
    );
    // A substantive one: queued, and I-3 is set aside.
    sb.ok(
        heron,
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-3",
            "--question",
            "daemon now or CLI-direct?",
            "--option",
            "daemon",
            "--option",
            "cli-direct",
            "--recommend",
            "1",
        ],
    );
    sb.ok(None, &["inbox"]);
    sb.refused(
        heron,
        &[
            "decide",
            "D-2",
            "--choose",
            "1",
            "--rationale",
            "agents do not take these",
        ],
    );
    sb.ok(
        None,
        &[
            "decide",
            "D-2",
            "--choose",
            "1",
            "--rationale",
            "fewer failure modes for agents",
        ],
    );
    sb.refused(
        None,
        &["decide", "D-2", "--choose", "0", "--rationale", "again"],
    );

    // Done needs evidence, then merge.
    sb.refused(heron, &["done", "I-1"]);
    fs::write(sb.dir.join("test-output.txt"), "running 3 tests\nok\n").unwrap();
    sb.refused(wren, &["done", "I-1", "--evidence", "test-output.txt"]);
    sb.ok(heron, &["done", "I-1", "--evidence", "test-output.txt"]);
    sb.ok(
        None,
        &[
            "merge",
            "I-1",
            "--commit",
            "0123456789abcdef0123456789abcdef01234567",
        ],
    );
    sb.refused(
        None,
        &[
            "merge",
            "I-1",
            "--commit",
            "0123456789abcdef0123456789abcdef01234567",
        ],
    );

    // I-2's dependency has merged.
    sb.ok(wren, &["claim", "I-2"]);
    sb.ok(
        wren,
        &[
            "send",
            "heron",
            "--kind",
            "question",
            "--about",
            "I-2",
            "is the fold's interface settled?",
        ],
    );
    sb.refused(wren, &["send", "wren", "--kind", "question", "to myself"]);
    sb.ok(heron, &["inbox"]);
    sb.ok(wren, &["inbox"]);

    sb.ok(None, &["report"]);
    sb.ok(None, &["verify"]);
    sb.ok(None, &["phase", "close"]);
    sb.refused(None, &["phase", "close"]);
    let r = sb.ok(None, &["--json", "report"]);
    let v: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["phase"]["state"], "closed");
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    assert_eq!(v["substantive"].as_array().unwrap().len(), 1);
    assert_eq!(v["mechanical"].as_array().unwrap().len(), 1);

    let got = normalise(&sb.transcript, &sb.dir);
    assert!(
        !got.contains("/tmp/") && !got.contains(&canary[..6]),
        "golden is clean"
    );
    let golden = golden_path();
    if std::env::var_os("UPDATE_GOLDEN").is_some() || !golden.exists() {
        fs::create_dir_all(golden.parent().unwrap()).unwrap();
        fs::write(&golden, &got).unwrap();
    }
    let want = fs::read_to_string(&golden).unwrap();
    if got != want {
        let actual = golden.with_extension("actual.txt");
        fs::write(&actual, &got).unwrap();
        panic!(
            "transcript differs from {}; actual written to {}",
            golden.display(),
            actual.display()
        );
    }
}

#[test]
fn every_help_fits_one_screen() {
    let subcommands: &[&[&str]] = &[
        &[],
        &["init"],
        &["phase"],
        &["phase", "open"],
        &["item", "add"],
        &["ready"],
        &["claim"],
        &["decision", "raise"],
        &["decide"],
        &["done"],
        &["merge"],
        &["inbox"],
        &["send"],
        &["report"],
        &["verify"],
        &["log"],
    ];
    for sub in subcommands {
        let out = Command::new(env!("CARGO_BIN_EXE_ris"))
            .args(*sub)
            .arg("--help")
            .output()
            .unwrap();
        assert!(out.status.success(), "{sub:?} --help");
        let lines = String::from_utf8(out.stdout).unwrap().lines().count();
        assert!(
            lines <= 40,
            "`ris {} --help` is {lines} lines",
            sub.join(" ")
        );
    }
}
