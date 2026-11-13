use super::*;

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
fn a_seat_that_picks_up_by_the_rules_passes() {
    let log = [
        r#"{"type":"system","subtype":"init"}"#.to_owned(),
        bash("ris inbox; ris ready"),
        bash("ris report"),
        bash("ris claim I-1"),
        read("docs/quickstart.md"),
    ]
    .join("\n");
    assert_eq!(cold_start(&log), Vec::<String>::new());
}

#[test]
fn each_fault_is_named() {
    let has = |log: &[String], want: &str| {
        let f = cold_start(&log.join("\n"));
        assert!(f.iter().any(|x| x.contains(want)), "{want} in {f:?}");
    };
    has(
        &[bash("ris claim I-1"), bash("ris inbox"), bash("ris ready")],
        "not inbox, ready and report",
    );
    has(
        &[
            read("PLAN.md"),
            bash("ris inbox"),
            bash("ris ready"),
            bash("ris report"),
        ],
        "used Read before",
    );
    has(
        &[
            bash("ris inbox && ris ready && ris report"),
            bash("cat docs/status.md"),
            bash("ris claim I-2"),
        ],
        "read a document with Bash",
    );
    has(
        &[
            bash("ris inbox; ris ready; ris report"),
            bash("RIS_AGENT=person ris decide D-1 --choose 0 --rationale x"),
        ],
        "set RIS_AGENT itself",
    );
    has(
        &[bash("ris inbox; ris ready; ris report"), bash("ris up")],
        "ran a person's command: ris up",
    );
    has(
        &[
            bash("ris inbox; ris ready; ris report"),
            bash("sqlite3 .osiris/store.sqlite 'select 1'"),
        ],
        "opened the store directly",
    );
    assert_eq!(cold_start("not json\n"), vec!["the run used no tools"]);
}
