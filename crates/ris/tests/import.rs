//! `ris import`: all or nothing, scanned, decisions as the person's.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = ris(dir, None, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_backlog_imports_whole_or_not_at_all() {
    let repo = std::env::temp_dir().join(format!("ris-import-{}", std::process::id()));
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    ok(&repo, &["init"]);
    let config = repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace("denylist = []", "denylist = [\"example client\"]"),
    )
    .unwrap();
    ok(&repo, &["phase", "open", "import"]);

    let good = repo.join("good.toml");
    fs::write(
        &good,
        r#"
[[item]]
key = "store"
goal = "A store"
checks = ["test"]

[[item]]
goal = "A cli over the store"
depends = ["store"]

[[decision]]
question = "Which engine?"
options = ["sqlite", "rocksdb"]
answer = 0
rationale = "No install"

[[decision]]
question = "Daemon now?"
options = ["yes", "no"]
other = "later, with leases"
rationale = "Fewer moving parts"

[[decision]]
question = "Which runner first?"
options = ["runner-a", "runner-b"]
recommend = 1
rationale = "Open in the plan; runner-b has the simpler adapter"
"#,
    )
    .unwrap();

    // A dry run writes nothing.
    let out = ok(&repo, &["import", "--dry-run", good.to_str().unwrap()]);
    assert!(
        out.contains("2 item(s) and 3 decision(s), 1 of them open"),
        "{out}"
    );
    assert!(ok(&repo, &["ready"]).contains("nothing ready"));

    // Decisions are the person's.
    let r = ris(&repo, Some("heron"), &["import", good.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));

    let out = ok(&repo, &["import", good.to_str().unwrap()]);
    assert!(
        out.contains("items I-1 to I-2 and decisions D-1 to D-3; 1 open"),
        "{out}"
    );
    // The open question waits for the person, recommendation intact,
    // and can be decided like any other.
    let inbox = ok(&repo, &["--json", "inbox"]);
    let v: serde_json::Value =
        serde_json::from_str::<serde_json::Value>(&inbox).unwrap()["data"].clone();
    let waiting = v["decisions"].as_array().unwrap();
    assert_eq!(waiting.len(), 1, "{inbox}");
    assert_eq!(waiting[0]["id"], "D-3", "{inbox}");
    assert_eq!(waiting[0]["recommended"], 1, "{inbox}");
    ok(
        &repo,
        &["decide", "D-3", "--choose", "1", "--rationale", "agreed"],
    );
    let i2 = ok(&repo, &["show", "I-2"]);
    assert!(i2.contains("depends on: I-1"), "{i2}");
    let d2 = ok(&repo, &["show", "D-2"]);
    assert!(
        d2.contains("later, with leases") && d2.contains("taken by the person"),
        "{d2}"
    );

    // The same file again: everything in it is already recorded, so
    // nothing is written, and each duplicate names what it duplicates.
    // A reworded goal differing only in case, spacing and a full stop
    // still counts as the same.
    let again = fs::read_to_string(&good)
        .unwrap()
        .replace("goal = \"A store\"", "goal = \"a   STORE.\"");
    fs::write(&good, again).unwrap();
    let r = ris(
        &repo,
        None,
        &["import", "--dry-run", good.to_str().unwrap()],
    );
    assert_eq!(r.status.code(), Some(1));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(err.contains("5 problem(s)"), "{err}");
    assert!(
        err.contains("item 1: the same goal as I-1")
            && err.contains("decision 3: the same question as D-3"),
        "{err}"
    );

    // One bad entry anywhere: nothing is written, every problem is
    // named, and the offending text is not echoed. The canary is
    // assembled at runtime: the exit scan has no exceptions.
    let canary = ["/Us", "ers/someone/x.rs"].concat();
    let bad = repo.join("bad.toml");
    fs::write(
        &bad,
        format!(
            "[[item]]\ngoal = \"fine\"\n\n[[item]]\ngoal = \"see {canary}\"\n\n[[item]]\ngoal = \"ok\"\nchecks = [\"nope\"]\ndepends = [\"missing\"]\n"
        ),
    )
    .unwrap();
    let r = ris(&repo, None, &["import", bad.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(err.contains("3 problem(s)"), "{err}");
    assert!(
        err.contains("item 2") && err.contains("absolute path"),
        "{err}"
    );
    assert!(err.contains("`nope`") && err.contains("`missing`"), "{err}");
    assert!(!err.contains(&canary[..10]), "{err}");
    let report = ok(&repo, &["--json", "report"]);
    let v: serde_json::Value =
        serde_json::from_str::<serde_json::Value>(&report).unwrap()["data"].clone();
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        2,
        "nothing from the bad file"
    );
}

#[test]
fn an_open_question_needs_a_recommendation_and_cannot_be_half_answered() {
    let repo = std::env::temp_dir().join(format!("ris-import-open-{}", std::process::id()));
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    ok(&repo, &["init"]);
    let config = repo.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace(
            "denylist = []",
            "confirm_empty_denylist = true\ndenylist = []",
        ),
    )
    .unwrap();
    ok(&repo, &["phase", "open", "import"]);
    let bad = repo.join("bad.toml");
    fs::write(
        &bad,
        r#"
[[decision]]
question = "No recommendation"
options = ["a", "b"]
rationale = "open"

[[decision]]
question = "Both answers"
options = ["a", "b"]
answer = 0
other = "c"
rationale = "x"

[[decision]]
question = "Recommendation out of range"
options = ["a", "b"]
recommend = 2
rationale = "open"
"#,
    )
    .unwrap();
    let r = ris(&repo, None, &["import", "--dry-run", bad.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(err.contains("3 problem(s)"), "{err}");
    assert!(
        err.contains("decision 1") && err.contains("needs `recommend`"),
        "{err}"
    );
    assert!(
        err.contains("decision 2") && err.contains("not both"),
        "{err}"
    );
    assert!(
        err.contains("decision 3") && err.contains("recommend 2"),
        "{err}"
    );
}
