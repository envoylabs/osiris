//! The advisor role: a seat beside the person that reads, adds items,
//! sends messages and raises decisions for the person, and is refused
//! everything else, before anything is written.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn ok(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "{agent}: ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn an_advisor_adds_sends_and_raises_and_does_nothing_else() {
    let dir = std::env::temp_dir().join(format!("ris-advisor-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, "person", &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace(
                "denylist = []",
                "denylist = [\"example client\"]\ndocs_item = false",
            )
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    ok(
        &dir,
        "person",
        &["seat", "add", "kite", "--role", "advisor"],
    );
    let status = ok(&dir, "person", &["--json", "status"]);
    assert!(status.contains("\"advisor\""), "{status}");
    ok(&dir, "person", &["phase", "open", "advice"]);

    // What it may do.
    for args in [
        &["ready"][..],
        &["inbox"],
        &["summary"],
        &["report"],
        &["status"],
        &["verify"],
        &["audit"],
        &["item", "add", "the parser drops trailing blank lines"],
        &["send", "heron", "--kind", "question", "is I-1 settled?"],
    ] {
        ok(&dir, "kite", args);
    }
    let raised = ok(
        &dir,
        "kite",
        &[
            "decision",
            "raise",
            "--substantive",
            "--question",
            "split the parser?",
            "--option",
            "yes",
            "--option",
            "no",
        ],
    );
    assert!(raised.contains("D-1 queued for the person"), "{raised}");
    ok(
        &dir,
        "kite",
        &[
            "decision",
            "explain",
            "D-1",
            "--summary",
            "s",
            "--pro",
            "0: a",
            "--con",
            "1: b",
            "--why",
            "w",
        ],
    );
    ok(&dir, "kite", &["show", "I-1"]);
    ok(&dir, "kite", &["eli5", "D-1"]);

    // What it may not, each refused with exit 2 and nothing written.
    let head = ok(&dir, "person", &["verify"]);
    for args in [
        &["claim", "I-1"][..],
        &["done", "I-1"],
        &["merge", "I-1"],
        &["release", "I-1"],
        &["audit", "I-1"],
        &["decide", "D-1", "--choose", "0", "--rationale", "r"],
        &[
            "decision",
            "raise",
            "--question",
            "q",
            "--option",
            "a",
            "--option",
            "b",
        ],
        &["check"],
        &["phase", "close"],
        &["item", "supersede", "I-1", "--by", "I-1"],
        &["action", "raise", "x", "--done-when", "y"],
        &["worktree", "kite"],
        &["up"],
    ] {
        let r = ris(&dir, "kite", args);
        assert_eq!(r.status.code(), Some(2), "ris {}", args.join(" "));
        let err = String::from_utf8_lossy(&r.stderr);
        assert!(
            err.contains("kite is an advisor, and an advisor does not"),
            "ris {}: {err}",
            args.join(" ")
        );
        assert!(err.contains("ris decision raise --substantive"), "{err}");
    }
    assert_eq!(ok(&dir, "person", &["verify"]), head);

    // In JSON the refusal names the role.
    let r = ris(&dir, "kite", &["--json", "claim", "I-1"]);
    let v: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "role_forbids");
    assert_eq!(v["error"]["role"], "advisor");

    // A lead still claims what the advisor added.
    ok(&dir, "heron", &["claim", "I-1"]);
}
