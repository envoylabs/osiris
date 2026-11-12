//! The three-seat drill: a lead and a builder, each in its own
//! worktree, and an auditor reading the main checkout, working at once
//! through ris on one shared store. Claims overlap, a substantive
//! decision sets the builder's item aside until the person takes it, a
//! dependency waits for its merge, the lead's and the builder's merges
//! race on two threads, the auditor reviews and sends a finding, and
//! every seat is refused outside its role: the auditor writing, an
//! agent taking a substantive decision, auditing its own work or
//! recording a person action done. The phase ends with every item
//! merged and reviewed and the chain verified.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
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

fn ok(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "{agent}: ris {} failed: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Refused with exit 2, the code for a refusal or a role's limit.
fn refused(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{agent}: ris {} should be refused: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).unwrap()
}

fn commit(dir: &Path, file: &str, item: &str, agent: &str) {
    fs::write(dir.join(file), format!("{file}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {file}"),
            "-m",
            &format!("Item: {item}\nAgent: {agent}"),
        ],
    );
}

#[test]
fn a_lead_a_builder_and_an_auditor_finish_a_phase_together() {
    let base = std::env::temp_dir().join(format!("ris-three-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main: PathBuf = base.join("repo");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    fs::write(main.join("README"), "base\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    ok(&main, "person", &["init", "--local"]);
    fs::write(
        main.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        main.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\n\
         [agents.heron]\nrole = \"lead\"\n\
         [agents.rook]\nrole = \"builder\"\n\
         [agents.wren]\nrole = \"auditor\"\n",
    )
    .unwrap();
    ok(&main, "person", &["phase", "open", "three seats"]);
    ok(&main, "person", &["item", "add", "the store"]);
    ok(&main, "person", &["item", "add", "the cli"]);
    ok(
        &main,
        "person",
        &["item", "add", "the report", "--depends", "I-1"],
    );
    // Each writing seat works in its own worktree, the lead included;
    // the main checkout stays on the main line and the auditor reads.
    ok(&main, "person", &["worktree", "heron"]);
    ok(&main, "person", &["worktree", "rook"]);
    let rook = base.join("repo-rook");
    let heron_dir = base.join("repo-heron");
    let (heron, wren) = (heron_dir.as_path(), main.as_path());

    // Lead and builder hold items at once, in their own places.
    ok(heron, "heron", &["claim", "I-1"]);
    let brief = ok(&rook, "rook", &["claim", "I-2"]);
    assert!(brief.contains("you are builder"), "{brief}");
    let status = ok(&main, "person", &["status"]);
    assert!(
        status.contains("holds I-1") && status.contains("holds I-2"),
        "{status}"
    );

    // The auditor never writes.
    for args in [&["claim", "I-3"][..], &["done", "I-1"], &["merge", "I-1"]] {
        refused(wren, "wren", args);
    }

    // The builder meets something expensive to reverse: it is queued,
    // its item set aside, and the dependent item still waits on I-1.
    ok(
        &rook,
        "rook",
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-2",
            "--question",
            "Which wire format?",
            "--option",
            "json",
            "--option",
            "binary",
            "--recommend",
            "0",
        ],
    );
    refused(&rook, "rook", &["claim", "I-3"]);
    refused(
        &rook,
        "rook",
        &["decide", "D-1", "--choose", "0", "--rationale", "mine"],
    );
    assert!(ok(&main, "person", &["summary"]).contains("D-1  open  Which wire format?"));
    ok(
        &main,
        "person",
        &[
            "decide",
            "D-1",
            "--choose",
            "0",
            "--rationale",
            "readable first",
        ],
    );
    ok(&rook, "rook", &["claim", "I-2"]);

    // Only the person records a person action done; an agent is
    // refused, and the action stays open.
    ok(
        &main,
        "person",
        &[
            "action",
            "raise",
            "add a fourth seat",
            "--done-when",
            "ris status lists it",
            "--informs",
            "I-2",
        ],
    );
    for (dir, agent) in [(heron, "heron"), (rook.as_path(), "rook"), (wren, "wren")] {
        refused(dir, agent, &["action", "done", "A-1", "--note", "done"]);
    }
    assert!(ok(&main, "person", &["show", "A-1"]).contains("open"));

    // Lead and builder finish together, and their merges race on two
    // threads for the one phase branch. Each lands, or the one that
    // loses is refused cleanly and lands when run again.
    ok(heron, "heron", &["done", "I-1"]);
    commit(heron, "store.txt", "I-1", "heron");
    ok(&rook, "rook", &["done", "I-2"]);
    commit(&rook, "cli.txt", "I-2", "rook");
    let race: Vec<(PathBuf, &str, &str)> = vec![
        (heron.to_path_buf(), "heron", "I-1"),
        (rook.clone(), "rook", "I-2"),
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(race.len()));
    let handles: Vec<_> = race
        .into_iter()
        .map(|(dir, agent, item)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (dir.clone(), agent, item, ris(&dir, agent, &["merge", item]))
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        results.iter().any(|(.., out)| out.status.success()),
        "one merge lands"
    );
    for (dir, agent, item, out) in &results {
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(
                matches!(out.status.code(), Some(1 | 2)) && !err.trim().is_empty(),
                "{agent}'s losing merge says why: {err}"
            );
            ok(dir, agent, &["merge", item]);
        }
    }
    // An agent never audits its own work, lead or builder.
    refused(heron, "heron", &["audit", "I-1"]);
    refused(&rook, "rook", &["audit", "I-2"]);

    // The auditor reviews both, and sends the builder a finding.
    let queue = ok(wren, "wren", &["audit"]);
    assert!(queue.contains("I-1") && queue.contains("I-2"), "{queue}");
    ok(
        wren,
        "wren",
        &[
            "send",
            "rook",
            "--kind",
            "finding",
            "--about",
            "I-2",
            "the cli has no help text",
        ],
    );
    ok(wren, "wren", &["audit", "I-1"]);
    ok(wren, "wren", &["audit", "I-2"]);
    assert!(ok(&rook, "rook", &["inbox"]).contains("the cli has no help text"));

    // I-1 has merged, so the dependent item is free; the builder takes
    // it, and the auditor reviews it too.
    ok(&rook, "rook", &["claim", "I-3"]);
    ok(&rook, "rook", &["done", "I-3"]);
    commit(&rook, "report.txt", "I-3", "rook");
    ok(&rook, "rook", &["merge", "I-3"]);
    ok(wren, "wren", &["audit", "I-3"]);

    // The phase's record: everything merged and reviewed, the chain
    // intact, and every file on the phase branch.
    let report = ok(&main, "person", &["report"]);
    for i in ["I-1", "I-2", "I-3"] {
        assert!(report.contains(&format!("{i}   merged")), "{report}");
    }
    assert!(report.contains("audited: 3 of 3"), "{report}");
    ok(&main, "person", &["verify"]);
    for f in ["store.txt", "cli.txt", "report.txt"] {
        let on = Command::new("git")
            .args(["cat-file", "-e", &format!("phase/P-1:{f}")])
            .current_dir(&main)
            .status()
            .unwrap();
        assert!(on.success(), "{f} is on the phase branch");
        assert!(!main.join(f).exists(), "the main checkout is untouched");
    }
}
