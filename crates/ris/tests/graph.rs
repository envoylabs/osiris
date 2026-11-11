//! The graph on `ris summary`: every edge of the work in motion is
//! drawn exactly once, hard and soft edges are told apart, and lines
//! fit the width.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent.unwrap_or("person"))
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
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

fn raise_substantive(dir: &Path, about: &str) {
    ok(
        dir,
        Some("heron"),
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            about,
            "--question",
            "which way",
            "--option",
            "a",
            "--option",
            "b",
        ],
    );
}

/// Read the drawing back into `(parent, child, kind)` edges, using the
/// indentation: a child sits four columns right of its parent.
fn edges_drawn(text: &str) -> Vec<(String, String, String)> {
    let section = text
        .split("== what waits on what ==")
        .nth(1)
        .unwrap()
        .split("== report ==")
        .next()
        .unwrap();
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in section.lines().filter(|l| !l.trim().is_empty()) {
        let chars: Vec<char> = line.chars().collect();
        let Some(start) = chars
            .iter()
            .position(|c| *c == 'I' || *c == 'D')
            .filter(|&p| chars.get(p + 1) == Some(&'-'))
        else {
            continue;
        };
        let id: String = chars[start..]
            .iter()
            .take_while(|c| !c.is_whitespace())
            .collect();
        let depth = start / 4;
        stack.truncate(depth);
        if depth > 0 {
            let kind = if chars[start - 3] == '╌' {
                "informs"
            } else {
                "blocks"
            };
            out.push((stack[depth - 1].clone(), id.clone(), kind.to_owned()));
        }
        stack.push(id);
    }
    out
}

#[test]
fn every_edge_is_drawn_once_and_matches_the_json() {
    let dir = std::env::temp_dir().join(format!("ris-graph-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
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
    ok(&dir, None, &["phase", "open", "graph"]);
    let long = "a goal long enough that its line is cut at the edge of the screen ".repeat(2);
    ok(&dir, None, &["item", "add", &long]);
    ok(&dir, None, &["item", "add", "second", "--depends", "I-1"]);
    ok(
        &dir,
        None,
        &[
            "item",
            "add",
            "third",
            "--depends",
            "I-1",
            "--depends",
            "I-2",
        ],
    );
    ok(&dir, None, &["item", "add", "to be replaced"]);
    ok(&dir, None, &["item", "add", "the replacement"]);
    ok(&dir, None, &["item", "add", "alone"]);
    // D-1 blocks I-1; D-2 blocked I-4, which I-5 replaced: it informs I-5.
    ok(&dir, Some("heron"), &["claim", "I-1"]);
    raise_substantive(&dir, "I-1");
    ok(&dir, Some("heron"), &["claim", "I-4"]);
    raise_substantive(&dir, "I-4");
    ok(&dir, None, &["item", "supersede", "I-4", "--by", "I-5"]);

    let text = ok(&dir, None, &["summary"]);
    let mut drawn = edges_drawn(&text);
    let out = ris(&dir, None, &["--json", "summary"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut want: Vec<(String, String, String)> = v["data"]["graph"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let unique: BTreeSet<_> = drawn.iter().cloned().collect();
    assert_eq!(unique.len(), drawn.len(), "an edge drawn twice:\n{text}");
    drawn.sort();
    want.sort();
    assert_eq!(drawn, want, "{text}");
    let s = |a: &str, b: &str, k: &str| (a.to_owned(), b.to_owned(), k.to_owned());
    assert_eq!(
        want,
        vec![
            s("D-1", "I-1", "blocks"),
            s("D-2", "I-5", "informs"),
            s("I-1", "I-2", "blocks"),
            s("I-1", "I-3", "blocks"),
            s("I-2", "I-3", "blocks"),
        ]
    );
    // I-3 has two parents: drawn once, marked under the other.
    assert!(text.contains("I-3  (above)"), "{text}");
    assert_eq!(v["data"]["graph"]["unlinked"], 1, "I-6 has no links");
    assert!(text.contains("1 open decision(s) and unmerged item(s) have no links"));
    let section = text.split("== what waits on what ==").nth(1).unwrap();
    let section = section.split("== report ==").next().unwrap();
    for line in section.lines() {
        assert!(line.chars().count() <= 80, "too wide: {line}");
    }
    assert!(text.contains('…'), "the long goal is cut");
}

/// A supersede that would make the replacement wait on itself is
/// refused, so the graph never holds a cycle: directly (I-2 depends on
/// I-1, which it would replace) and through another item (I-3 depends
/// on I-2, which depends on I-1).
#[test]
fn a_supersede_that_closes_a_cycle_is_refused() {
    let dir = std::env::temp_dir().join(format!("ris-graph-cycle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
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
    ok(&dir, None, &["phase", "open", "cycles"]);
    ok(&dir, None, &["item", "add", "first"]);
    ok(&dir, None, &["item", "add", "second", "--depends", "I-1"]);
    ok(&dir, None, &["item", "add", "third", "--depends", "I-2"]);
    ok(&dir, None, &["item", "add", "unrelated"]);
    for by in ["I-2", "I-3"] {
        let out = ris(&dir, None, &["item", "supersede", "I-1", "--by", by]);
        assert_eq!(out.status.code(), Some(2), "supersede I-1 by {by}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(&format!("{by} cannot replace I-1")), "{err}");
    }
    // A replacement outside the chain is still accepted.
    ok(&dir, None, &["item", "supersede", "I-1", "--by", "I-4"]);
    let ready = ok(&dir, Some("heron"), &["ready"]);
    assert!(ready.contains("I-4"), "{ready}");
    let text = ok(&dir, None, &["summary"]);
    assert!(text.contains("I-2  waiting"), "{text}");
}

/// An item with an unmerged dependency is `waiting` everywhere the
/// person reads it: the graph, the report's lines and counts, and
/// `ris show`, matching `ris ready`, which leaves it out. The JSON
/// keeps its stored state and adds the dependencies it waits on.
#[test]
fn a_waiting_item_has_one_state_on_every_screen() {
    let dir = std::env::temp_dir().join(format!("ris-graph-waiting-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
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
    ok(&dir, None, &["phase", "open", "waiting"]);
    ok(&dir, None, &["item", "add", "first"]);
    ok(&dir, None, &["item", "add", "second", "--depends", "I-1"]);

    let ready = ok(&dir, Some("heron"), &["ready"]);
    assert!(!ready.contains("I-2"), "{ready}");
    let text = ok(&dir, None, &["summary"]);
    let graph = text
        .split("== what waits on what ==")
        .nth(1)
        .unwrap()
        .split("== report ==")
        .next()
        .unwrap();
    assert!(graph.contains("I-2  waiting"), "{text}");
    let report = ok(&dir, None, &["report"]);
    assert!(report.contains("items: 1 ready, 1 waiting"), "{report}");
    let line = report.lines().find(|l| l.contains("I-2")).unwrap();
    assert!(line.contains("waiting"), "{report}");
    assert!(!line.contains("ready"), "{report}");
    let report_in_summary = text.split("== report ==").nth(1).unwrap();
    let line = report_in_summary
        .lines()
        .find(|l| l.contains("I-2"))
        .unwrap();
    assert!(line.contains("waiting"), "{text}");
    assert!(ok(&dir, None, &["show", "I-2"]).starts_with("I-2 waiting"));

    let out = ris(&dir, None, &["--json", "report"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let item = v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "I-2")
        .unwrap()
        .clone();
    assert_eq!(item["state"], "ready", "the stored state is unchanged");
    assert_eq!(item["waiting_on"], serde_json::json!(["I-1"]));
}
