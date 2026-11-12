//! `ris summary` covers the open phase by default: its decisions, its
//! graph, its items in the report and its ready items. Work carried
//! over from earlier phases is counted, with how to see it, and
//! `--all` shows everything. With no phase open, it says so and names
//! the next step.

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
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    serde_json::from_str::<Value>(&ok(dir, "person", &all)).unwrap()["data"].clone()
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn summary_shows_the_open_phase_and_counts_the_rest() {
    let dir = std::env::temp_dir().join(format!("ris-summary-scope-{}", std::process::id()));
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

    // P-1 leaves an item unstarted and one set aside on a decision.
    ok(&dir, "person", &["phase", "open", "first"]);
    ok(&dir, "person", &["item", "add", "left over"]);
    ok(&dir, "person", &["item", "add", "set aside"]);
    ok(&dir, "heron", &["claim", "I-2"]);
    ok(
        &dir,
        "heron",
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-2",
            "--question",
            "an old question",
            "--option",
            "a",
            "--option",
            "b",
        ],
    );
    ok(&dir, "person", &["phase", "close"]);

    // With no phase open, summary says so and names the next step.
    let text = ok(&dir, "person", &["summary"]);
    assert!(text.starts_with("no phase is open"), "{text}");
    assert!(text.contains("ris phase open"), "{text}");
    assert!(
        text.contains("2 item(s) and 1 open decision(s) from earlier phases are not shown"),
        "{text}"
    );
    assert!(text.contains("ris summary --all"), "{text}");
    let v = json(&dir, &["summary"]);
    assert!(ids(&v["ready"]["ready"]).is_empty(), "{v}");
    assert!(ids(&v["inbox"]["decisions"]).is_empty(), "{v}");

    // No empty sections stand in for a phase that is not there.
    assert!(!text.contains("=="), "{text}");
    assert_eq!(text.lines().count(), 4, "{text}");

    // P-2 with no item yet says so and how to add one.
    ok(&dir, "person", &["phase", "open", "second"]);
    let text = ok(&dir, "person", &["summary"]);
    assert!(text.starts_with("P-2 open: second\n"), "{text}");
    assert!(text.contains("no items in this phase yet"), "{text}");

    // P-2 has its own item; the summary shows only it.
    ok(&dir, "person", &["item", "add", "this phase's work"]);
    let text = ok(&dir, "person", &["summary"]);
    assert!(!text.contains("no phase is open"), "{text}");
    assert!(!text.contains("an old question"), "{text}");
    assert!(!text.contains("left over"), "{text}");
    assert!(
        text.starts_with("P-2 open: second\nitems: 1 ready\n\nI-3  ready  this phase's work\n--\n"),
        "{text}"
    );
    // The footer counts what is left out and names where it is.
    assert!(text.contains("1 open decision(s) in all"), "{text}");
    assert!(text.contains("`ris decide`"), "{text}");
    assert!(
        text.contains("2 item(s) and 1 open decision(s) from earlier phases are not shown"),
        "{text}"
    );
    assert!(text.contains("`ris report`"), "{text}");
    let v = json(&dir, &["summary"]);
    assert_eq!(v["all"], false);
    assert_eq!(v["carried_over"]["items"], 2);
    assert_eq!(v["carried_over"]["decisions"], 1);
    assert_eq!(ids(&v["ready"]["ready"]), ["I-3"]);
    assert_eq!(ids(&v["report"]["items"]), ["I-3"]);
    assert!(ids(&v["inbox"]["decisions"]).is_empty(), "{v}");
    assert_eq!(
        v["graph"]["nodes"],
        serde_json::json!([{ "id": "I-3", "state": "ready", "phase": "P-2" }])
    );

    // An item of the phase that waits on an earlier phase's item draws
    // that item, marked with its phase, and the open decision it is set
    // aside on, though an earlier phase raised it. The earlier phase's
    // other item stays out.
    ok(
        &dir,
        "person",
        &["item", "add", "builds on the old", "--depends", "I-2"],
    );
    let text = ok(&dir, "person", &["summary"]);
    let picture = text
        .split_once("\n\n")
        .unwrap()
        .1
        .split("\n--\n")
        .next()
        .unwrap();
    assert_eq!(
        picture,
        "D-1  open  an old question\n\
         └── I-2  blocked [P-1]  set aside\n    \
         └── I-4  waiting  builds on the old\n\
         I-3  ready  this phase's work",
        "{text}"
    );
    assert!(text.contains("items: 1 ready, 1 waiting\n"), "{text}");
    assert!(
        text.contains("1 item(s) and 0 open decision(s) from earlier phases are not shown"),
        "{text}"
    );
    let v = json(&dir, &["summary"]);
    assert_eq!(v["carried_over"]["items"], 1);
    assert_eq!(v["carried_over"]["decisions"], 0);
    assert_eq!(
        v["graph"]["edges"],
        serde_json::json!([
            { "from": "D-1", "to": "I-2", "kind": "blocks" },
            { "from": "I-2", "to": "I-4", "kind": "blocks" },
        ])
    );

    // Nothing else is on the picture: no report, no hashes, no queue of
    // decisions in full, no list of what is ready.
    for absent in [
        "==",
        "log:",
        "report:",
        "audited:",
        "messages:",
        "mechanical",
        "[substantive]",
        "answer:",
    ] {
        assert!(!text.contains(absent), "{absent:?} in:\n{text}");
    }

    // --all shows everything, as the commands on their own do.
    let text = ok(&dir, "person", &["summary", "--all"]);
    assert!(!text.contains("not shown"), "{text}");
    assert!(
        text.contains("an old question") && text.contains("left over"),
        "{text}"
    );
    let v = json(&dir, &["summary", "--all"]);
    assert_eq!(v["all"], true);
    assert_eq!(v["carried_over"]["items"], 0);
    assert_eq!(ids(&v["ready"]["ready"]), ["I-3", "I-1"]);
    assert_eq!(ids(&v["inbox"]["decisions"]), ["D-1"]);
    assert_eq!(v["report"], json(&dir, &["report"]));
    assert_eq!(v["ready"], json(&dir, &["ready"]));
    for section in [
        "== decisions ==",
        "== what waits on what ==",
        "== report ==",
        "== ready ==",
    ] {
        assert!(text.contains(section), "{section} in:\n{text}");
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
}

/// The picture keeps a merged item of the phase, with its state, and
/// the link to what depended on it: `met`, since nothing waits on it
/// any more. A claimed item names its holder.
#[test]
fn a_merged_item_stays_on_the_picture_with_its_link() {
    let dir = std::env::temp_dir().join(format!("ris-summary-merged-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, "person", &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        dir.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\nphase_branches = false\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    ok(&dir, "person", &["phase", "open", "merged"]);
    ok(&dir, "person", &["item", "add", "first"]);
    ok(
        &dir,
        "person",
        &["item", "add", "second", "--depends", "I-1"],
    );
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    fs::write(dir.join("first.txt"), "first\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "commit",
            "-q",
            "-m",
            "feat: first",
            "-m",
            "Item: I-1\nAgent: heron",
        ],
    );
    ok(&dir, "person", &["merge", "I-1"]);
    ok(&dir, "heron", &["claim", "I-2"]);

    let text = ok(&dir, "person", &["summary"]);
    assert!(
        text.starts_with(
            "P-1 open: merged\nitems: 1 claimed, 1 merged\n\n\
             I-1  merged  first\n\
             └── I-2  claimed (heron)  second\n--\n"
        ),
        "{text}"
    );
    let v = json(&dir, &["summary"]);
    assert_eq!(
        v["graph"]["edges"],
        serde_json::json!([{ "from": "I-1", "to": "I-2", "kind": "met" }])
    );
    // Every phase's graph is the work in motion: the merged item is not
    // part of it.
    let v = json(&dir, &["summary", "--all"]);
    assert_eq!(v["graph"]["edges"], serde_json::json!([]));
}
