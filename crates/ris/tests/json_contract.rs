//! The `--json` contract: every outcome, success or failure, is one
//! envelope on stdout carrying the format number; streams print one
//! envelope per line; the top-level keys of each command's data are
//! pinned here, so changing them is a deliberate format change.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn cmd(dir: &Path, agent: Option<&str>) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ris"));
    c.current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        c.env("RIS_AGENT", a);
    }
    c
}

fn run(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    cmd(dir, agent).args(args).output().unwrap()
}

/// Run with `--json`; the envelope, after checking its frame.
fn json(dir: &Path, agent: Option<&str>, args: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = run(dir, agent, &all);
    assert!(
        out.stderr.is_empty(),
        "ris --json {}: nothing on stderr, got {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("ris --json {}: {e}", args.join(" ")));
    assert_eq!(v["format"], 1, "{v}");
    let ok = v["ok"].as_bool().expect("ok is a boolean");
    assert_eq!(ok, out.status.success(), "ok matches the exit status: {v}");
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    if ok {
        assert_eq!(keys, ["data", "format", "ok"], "{v}");
    } else {
        assert_eq!(keys, ["error", "format", "ok"], "{v}");
        assert!(
            v["error"]["kind"].is_string() && v["error"]["message"].is_string(),
            "{v}"
        );
    }
    (out.status.code(), v)
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-json-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // A git repository, for `ris scan`; no commits, so no phase branches.
    assert!(
        Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success()
    );
    assert!(run(&dir, None, &["init"]).status.success());
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
    dir
}

#[test]
fn every_outcome_is_one_envelope_and_the_data_keys_are_pinned() {
    let dir = project("envelope");
    let heron = Some("heron");
    json(&dir, None, &["phase", "open", "contract"]);
    json(&dir, None, &["item", "add", "a task"]);
    json(&dir, heron, &["claim", "I-1"]);
    let (_, raised) = json(
        &dir,
        heron,
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-1",
            "--question",
            "q",
            "--option",
            "a",
            "--option",
            "b",
        ],
    );
    assert_eq!(raised["data"]["decision"], "D-1");

    // Pinned top-level keys of the data a client reads most.
    let pinned: &[(&[&str], &[&str])] = &[
        (&["ready"], &["ready"]),
        (&["inbox"], &["actions", "checks", "decisions"]),
        (
            &["summary"],
            &[
                "all",
                "carried_over",
                "checks",
                "graph",
                "inbox",
                "ready",
                "report",
            ],
        ),
        (&["status"], &["seats"]),
        (
            &["report"],
            &[
                "answered_outside_options",
                "items",
                "log",
                "mechanical",
                "merged_ahead",
                "messages",
                "phase",
                "previous_report",
                "report",
                "substantive",
                "tally",
            ],
        ),
        (
            // I-1 is blocked on D-1 here, so it carries what blocks it.
            &["show", "I-1"],
            &[
                "audited_by",
                "blocked_on",
                "check_runs",
                "checks",
                "decisions",
                "depends_on",
                "goal",
                "id",
                "phase",
                "state",
            ],
        ),
        (
            &["show", "D-1"],
            &[
                "about",
                "by",
                "class",
                "explained",
                "id",
                "options",
                "question",
                "raised_at_event",
                "recommended",
                "state",
            ],
        ),
        (&["eli5", "D-1"], &["decision", "explanation"]),
        (&["verify"], &["head", "len", "missing_text"]),
    ];
    for (args, want) in pinned {
        let (_, v) = json(&dir, None, args);
        assert_eq!(keys(&v["data"]), *want, "ris --json {}", args.join(" "));
    }

    // Actors are typed, not prose.
    let (_, d) = json(&dir, None, &["show", "D-1"]);
    assert_eq!(
        d["data"]["by"],
        serde_json::json!({ "kind": "agent", "seat": "heron" })
    );
    json(
        &dir,
        None,
        &["decide", "D-1", "--choose", "0", "--rationale", "ok"],
    );
    let (_, d) = json(&dir, None, &["show", "D-1"]);
    assert_eq!(
        d["data"]["taken_by"],
        serde_json::json!({ "kind": "person" })
    );

    // Failures: on stdout, with kind, message and next, and the exit
    // status unchanged.
    let (code, v) = json(&dir, Some("wren"), &["claim", "I-1"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["kind"], "role_forbids");
    assert_eq!(v["error"]["seat"], "wren");
    let (code, v) = json(&dir, heron, &["claim", "I-9"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["kind"], "refused");
    assert_eq!(
        v["error"]["refusal"],
        serde_json::json!({ "unknown_item": "I-9" })
    );
    assert!(v["error"]["next"].is_string());
    let (code, v) = json(&dir, None, &["show", "banana"]);
    assert_eq!(code, Some(1));
    assert_eq!(v["error"]["kind"], "other");

    // A leak names the place and category, never the text. The canary
    // is assembled at runtime: the exit scan has no exceptions.
    fs::write(
        dir.join("notes.md"),
        format!("see {}\n", ["/Us", "ers/someone/x"].concat()),
    )
    .unwrap();
    let (code, v) = json(&dir, None, &["scan", "notes.md"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["kind"], "leak");
    assert_eq!(v["error"]["hits"][0]["file"], "notes.md");
    assert_eq!(v["error"]["hits"][0]["line"], 1);
    assert!(!v.to_string().contains("someone"));
}

#[test]
fn streams_print_one_envelope_per_line_and_a_closed_reader_is_not_a_crash() {
    let dir = project("streams");
    let out = run(&dir, None, &["phase", "open", "streams"]);
    assert!(out.status.success());
    // Enough events that the log is larger than a pipe's buffer, so a
    // reader that hangs up early is noticed by the writer.
    let backlog: String = (0..400)
        .map(|n| {
            format!(
                "[[item]]\ngoal = \"task {n} with a goal long enough to fill the pipe quickly\"\n\n"
            )
        })
        .collect();
    fs::write(dir.join("backlog.toml"), backlog).unwrap();
    assert!(
        run(&dir, None, &["import", "backlog.toml"])
            .status
            .success()
    );
    let out = run(&dir, None, &["--json", "log"]);
    let lines: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 401);
    assert!(lines.iter().all(|l| l["format"] == 1 && l["ok"] == true));
    assert_eq!(lines[0]["data"]["structure"]["event"], "phase_opened");
    assert_eq!(lines[1]["data"]["structure"]["item"], "I-1");

    // Without --json, the log is bare event lines.
    let bare = run(&dir, None, &["log"]);
    let first: Value = serde_json::from_str(
        String::from_utf8(bare.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["seq"], 1);

    // Read one line and hang up, as `ris log | head -1` does.
    let mut child = cmd(&dir, None)
        .args(["log"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    drop(reader);
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert!(out.status.success(), "a closed reader ends ris quietly");
}
