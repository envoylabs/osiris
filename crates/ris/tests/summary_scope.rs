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

    // P-2 has its own item; the summary shows only it.
    ok(&dir, "person", &["phase", "open", "second"]);
    ok(&dir, "person", &["item", "add", "this phase's work"]);
    let text = ok(&dir, "person", &["summary"]);
    assert!(!text.contains("no phase is open"), "{text}");
    assert!(!text.contains("an old question"), "{text}");
    assert!(!text.contains("left over"), "{text}");
    assert!(text.contains("this phase's work"), "{text}");
    let v = json(&dir, &["summary"]);
    assert_eq!(v["all"], false);
    assert_eq!(v["carried_over"]["items"], 2);
    assert_eq!(v["carried_over"]["decisions"], 1);
    assert_eq!(ids(&v["ready"]["ready"]), ["I-3"]);
    assert_eq!(ids(&v["report"]["items"]), ["I-3"]);
    assert!(ids(&v["inbox"]["decisions"]).is_empty(), "{v}");
    assert!(v["graph"]["nodes"].as_array().unwrap().is_empty(), "{v}");

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
}
