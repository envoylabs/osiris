//! The brief `ris claim` prints: rules, then project, then item, so the
//! unchanging part comes first; the decisions that touch the item; and
//! a fixed size, reached by leaving out the oldest decisions.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-brief-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    ok(&dir, None, &["phase", "open", "the phase goal"]);
    dir
}

fn mechanical(dir: &Path, about: &str, question: &str) {
    ok(
        dir,
        Some("heron"),
        &[
            "decision",
            "raise",
            "--about",
            about,
            "--question",
            question,
            "--option",
            "a",
            "--option",
            "b",
            "--recommend",
            "1",
            "--rationale",
            "because b",
        ],
    );
}

#[test]
fn the_brief_puts_what_never_changes_first_and_carries_the_decisions_that_touch_the_item() {
    let dir = project("order");
    ok(&dir, None, &["item", "add", "the base"]);
    ok(&dir, None, &["item", "add", "unrelated"]);
    let heron = Some("heron");
    ok(&dir, heron, &["claim", "I-1"]);
    mechanical(&dir, "I-1", "a choice in the base");
    ok(&dir, heron, &["release", "I-1"]);
    ok(&dir, heron, &["claim", "I-2"]);
    mechanical(&dir, "I-2", "a choice elsewhere");
    ok(&dir, heron, &["release", "I-2"]);

    // A decision about a dependency also touches the item; the golden
    // transcript covers that case, where I-2 depends on a merged I-1.
    let brief = ok(&dir, heron, &["claim", "I-1"]);
    let at = |s: &str| {
        brief
            .find(s)
            .unwrap_or_else(|| panic!("{s:?} in:\n{brief}"))
    };
    assert!(at("rules:") < at("project: P-1 the phase goal"));
    assert!(at("project:") < at("item:    I-1 the base"));
    assert!(brief.contains("you are lead"));
    assert!(brief.contains("D-1 a choice in the base → b (because b)"));
    assert!(!brief.contains("a choice elsewhere"), "{brief}");

    // With no rules file, the brief names the built-in rules rather than
    // a file that is not there.
    ok(&dir, heron, &["release", "I-1"]);
    fs::remove_file(dir.join("OSIRIS.md")).unwrap();
    let brief = ok(&dir, heron, &["claim", "I-1"]);
    assert!(brief.contains("rules:   `ris rules`"), "{brief}");
    assert!(!brief.contains("OSIRIS.md"), "{brief}");
}

#[test]
fn the_brief_has_a_fixed_size_and_says_what_it_left_out() {
    let dir = project("size");
    ok(&dir, None, &["item", "add", "a busy item"]);
    let heron = Some("heron");
    ok(&dir, heron, &["claim", "I-1"]);
    let long = "a question long enough that sixty of them overflow the brief ".repeat(2);
    for n in 0..60 {
        mechanical(&dir, "I-1", &format!("{n}: {long}"));
    }
    ok(&dir, heron, &["release", "I-1"]);

    let brief = ok(&dir, heron, &["claim", "I-1"]);
    let body = brief.strip_prefix("claimed I-1\n").unwrap();
    assert!(body.len() <= ris::brief::BRIEF_MAX, "{} bytes", body.len());
    assert!(brief.contains("older decision(s): ris show I-1"), "{brief}");
    // Newest first: the last decision raised is kept, the first is cut.
    assert!(brief.contains("D-60 59:"), "{brief}");
    assert!(!brief.contains("D-1 0:"), "{brief}");

    ok(&dir, heron, &["release", "I-1"]);
    let out = ris(&dir, heron, &["--json", "claim", "I-1"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let data = &v["data"];
    let kept = data["decisions"].as_array().unwrap().len();
    let left = data["decisions_left_out"].as_u64().unwrap() as usize;
    assert_eq!(kept + left, 60);
    assert!(left > 0);
    assert_eq!(data["rules"]["path"], "OSIRIS.md");
    assert_eq!(data["project"]["role"], "lead");
}

#[test]
fn a_decision_about_a_superseded_item_is_in_its_replacements_brief() {
    let dir = project("supersede");
    ok(&dir, None, &["item", "add", "the old pause"]);
    ok(&dir, None, &["item", "add", "the old base"]);
    let heron = Some("heron");
    ok(&dir, heron, &["claim", "I-1"]);
    mechanical(&dir, "I-1", "what does pause act on");
    ok(&dir, heron, &["release", "I-1"]);
    ok(&dir, heron, &["claim", "I-2"]);
    mechanical(&dir, "I-2", "how is the base shaped");
    ok(&dir, heron, &["release", "I-2"]);
    ok(&dir, None, &["item", "add", "the new pause"]);
    ok(&dir, None, &["item", "add", "the new base"]);
    ok(
        &dir,
        None,
        &["item", "add", "on the old base", "--depends", "I-2"],
    );
    ok(&dir, None, &["item", "supersede", "I-1", "--by", "I-3"]);
    ok(&dir, None, &["item", "supersede", "I-2", "--by", "I-4"]);

    let brief = ok(&dir, heron, &["claim", "I-3"]);
    assert!(brief.contains("D-1 what does pause act on"), "{brief}");
    assert!(!brief.contains("how is the base shaped"), "{brief}");
    ok(&dir, heron, &["release", "I-3"]);

    // I-5 depends on I-2, which I-4 replaced: it waits on I-4, and the
    // brief for I-4 carries I-2's decision.
    let brief = ok(&dir, heron, &["claim", "I-4"]);
    assert!(brief.contains("D-2 how is the base shaped"), "{brief}");
    assert!(!brief.contains("what does pause act on"), "{brief}");
}
