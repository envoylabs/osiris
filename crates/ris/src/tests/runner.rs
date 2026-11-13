use super::*;

fn checks() -> ChecksConfig {
    ChecksConfig::parse("[checks.test]\ncommand = \"cargo test\"\n").unwrap()
}

#[test]
fn an_auditor_cannot_edit_and_a_builder_can_run_its_checks() {
    let a = allowlist(Role::Auditor, &checks(), &[]);
    assert!(a.contains(&"Bash(ris *)".to_owned()));
    assert!(
        !a.iter()
            .any(|t| t == "Edit" || t == "Write" || t == "Bash(git *)")
    );
    assert!(!a.contains(&"Bash(cargo test)".to_owned()));
    let b = allowlist(Role::Builder, &checks(), &["Bash(cargo *)".to_owned()]);
    for t in [
        "Edit",
        "Write",
        "Bash(git *)",
        "Bash(cargo test)",
        "Bash(cargo *)",
    ] {
        assert!(b.contains(&t.to_owned()), "{t}");
    }
    // Additions add; they never duplicate.
    let again = allowlist(Role::Builder, &checks(), &["Edit".to_owned()]);
    assert_eq!(again.iter().filter(|t| *t == "Edit").count(), 1);
}

#[test]
fn the_sandbox_has_no_way_out() {
    let s: serde_json::Value =
        serde_json::from_str(&sandbox_settings(&["x".into()], &["y".into()], &[])).unwrap();
    assert_eq!(s["sandbox"]["filesystem"]["denyWrite"], json!(["y"]));
    assert_eq!(s["sandbox"]["enabled"], true);
    assert_eq!(s["sandbox"]["failIfUnavailable"], true);
    assert_eq!(s["sandbox"]["allowUnsandboxedCommands"], false);
    assert_eq!(s["sandbox"]["network"]["allowedDomains"], json!([]));
}

#[test]
fn claude_is_refused_rather_than_asked() {
    let args = claude_args(
        "go",
        &["Read".into(), "Bash(ris *)".into()],
        &["Edit(//r/c)".into()],
        "{}",
        Some("m"),
    );
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--disallowedTools" && w[1] == "Edit(//r/c)")
    );
    let has = |a: &str, b: &str| args.windows(2).any(|w| w[0] == a && w[1] == b);
    assert!(has("-p", "go"));
    assert!(has("--permission-mode", "dontAsk"));
    assert!(has("--permission-prompts", "none"));
    assert!(has("--allowedTools", "Read,Bash(ris *)"));
    assert!(has("--settings", "{}"));
    assert!(has("--model", "m"));
    assert!(!args.iter().any(|a| a.contains("skip-permissions")));
}

#[test]
fn a_stream_reads_as_what_was_said_and_done() {
    let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"claiming I-3"},{"type":"tool_use","name":"Bash","input":{"command":"ris claim I-3"}}]}}"#;
    assert_eq!(
        render_line(said).unwrap(),
        "claiming I-3\n→ Bash ris claim I-3"
    );
    assert_eq!(render_line(r#"{"type":"system","subtype":"init"}"#), None);
    assert_eq!(
        render_line(r#"{"type":"result","subtype":"success"}"#).unwrap(),
        "== success =="
    );
    assert_eq!(render_line("plain text").unwrap(), "plain text");
}

/// The last activity is the last thing a run did, skipping
/// bookkeeping records, and is cut to one short line.
#[test]
fn the_last_activity_is_the_last_thing_said_or_run() {
    let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"claiming I-3"},{"type":"tool_use","name":"Bash","input":{"command":"ris claim I-3"}}]}}"#;
    let user = r#"{"type":"user","message":{}}"#;
    let run = format!("{said}\n{user}\n");
    assert_eq!(
        last_activity(&run, 80).as_deref(),
        Some("→ Bash ris claim I-3")
    );
    assert_eq!(last_activity(&run, 8).as_deref(), Some("→ Bash …"));
    assert_eq!(last_activity(user, 80), None);
    assert_eq!(last_activity("", 80), None);
}

/// A record still being written at the end of a live run file is
/// not shown: the activity is the last complete record's.
#[test]
fn a_half_written_last_record_is_not_the_activity() {
    let said = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ris claim I-3"}}]}}"#;
    let partial = r#"{"type":"assistant","message":{"content":[{"ty"#;
    let run = format!("{said}\n{partial}");
    assert_eq!(
        last_activity(&run, 80).as_deref(),
        Some("→ Bash ris claim I-3")
    );
    assert_eq!(last_activity(partial, 80), None);
}

/// A seat's latest run is its highest-numbered run file; another
/// seat's files, and names that only start the same, are not it.
#[test]
fn the_latest_run_is_the_highest_numbered() {
    let dir = std::env::temp_dir().join(format!("ris-latest-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(latest_run(&dir, "rook"), None);
    for f in [
        "rook-2.jsonl",
        "rook-10.jsonl",
        "rooky-99.jsonl",
        "heron-50.jsonl",
    ] {
        std::fs::write(dir.join(f), "").unwrap();
    }
    assert_eq!(latest_run(&dir, "rook"), Some(dir.join("rook-10.jsonl")));
    assert_eq!(latest_run(&dir.join("absent"), "rook"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A line that reaches the file in two writes is read once, whole,
/// after its second write; a last line without a newline is read
/// only when the writer has finished.
#[test]
fn a_line_written_in_two_parts_is_read_once_whole() {
    let line = r#"{"type":"result","subtype":"success"}"#;
    let (half, rest) = line.split_at(10);
    let mut file = String::from("first\n");
    file.push_str(half);
    let (got, at) = lines_from(&file, 0, false);
    assert_eq!(got, ["first"]);
    assert_eq!(at, 6);
    file.push_str(rest);
    file.push('\n');
    let (got, at) = lines_from(&file, at, false);
    assert_eq!(got, [line]);
    assert_eq!(at, file.len());
    assert_eq!(lines_from(&file, at, false), (vec![], file.len()));
    // The writer stopped mid-line: the end is read as it is.
    file.push_str("cut");
    assert_eq!(lines_from(&file, at, false).0, Vec::<&str>::new());
    assert_eq!(lines_from(&file, at, true), (vec!["cut"], file.len()));
}

#[test]
fn the_files_that_set_limits_are_denied_by_absolute_path() {
    let p = protected(
        Path::new("/r"),
        Path::new("/r/.osiris/OSIRIS.md"),
        Path::new("/r-rook"),
    );
    let d = deny_edits(&p);
    for want in [
        "Edit(//r/.osiris/config.toml)",
        "Edit(//r/.osiris/checks.toml)",
        "Edit(//r/.osiris/OSIRIS.md)",
        "Edit(//r-rook/CLAUDE.local.md)",
    ] {
        assert!(d.contains(&want.to_owned()), "{want} in {d:?}");
    }
    // Every rules file an agent loads, in both places, whatever the
    // rules path is and whether or not the files exist.
    let p = protected(
        Path::new("/r"),
        Path::new("/r/OSIRIS.md"),
        Path::new("/r-rook"),
    );
    for base in ["/r", "/r-rook"] {
        for f in RULES_FILES {
            let want = Path::new(base).join(f);
            assert!(p.contains(&want), "{} in {p:?}", want.display());
        }
    }
    assert!(p.contains(&PathBuf::from("/r/.osiris/OSIRIS.md")));
    let mut unique = p.clone();
    unique.dedup();
    assert_eq!(unique, p, "each path once");
}

#[test]
fn each_role_is_told_when_to_stop() {
    for role in Role::ALL.iter().copied() {
        let p = prompt("rook", role);
        assert!(p.contains("RIS_AGENT is set to rook"), "{p}");
        // What the cold-start drill (crate::drill) holds a seat to.
        assert!(
            p.contains("`ris inbox`, `ris ready` and `ris report`, run first"),
            "{p}"
        );
        assert!(p.contains("refused because you are paused"), "{p}");
        assert!(!p.contains("person action"), "{p}");
    }
    assert!(prompt("wren", Role::Auditor).contains("Never write to the tree"));
    assert!(prompt("kite", Role::Advisor).contains("never write to the tree"));
}

#[test]
fn writers_are_told_the_decision_rule_and_the_auditor_its_scope() {
    for role in [Role::Lead, Role::Builder] {
        let p = prompt("rook", role);
        assert!(
            p.contains("(a new event, a new JSON key) is mechanical"),
            "{p}"
        );
        assert!(p.contains("renaming"), "{p}");
        // The four-step process: a queued decision waits for the
        // review and never stops the seat.
        assert!(p.contains("never stops you: take another item"), "{p}");
        assert!(p.contains("person's review"), "{p}");
    }
    // No prompt sends a seat to wait for a check-in.
    for role in Role::ALL.iter().copied() {
        assert!(!prompt("rook", role).contains("check-in"));
    }
    let p = prompt("wren", Role::Auditor);
    assert!(p.contains("the open phase's items"), "{p}");
}

#[test]
fn an_advisor_reads_and_runs_ris_and_nothing_more() {
    let a = allowlist(Role::Advisor, &checks(), &[]);
    assert_eq!(a, READ.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    let g = GitDirs {
        common: PathBuf::from("/r/.git"),
        own: None,
    };
    let w = writable(Path::new("/r"), Role::Advisor, Some(&g));
    assert_eq!(w, vec![PathBuf::from("/r/.osiris")]);
    for role in Role::ALL.iter().copied() {
        assert_eq!(Role::parse(role.as_str()), Some(role));
    }
}

#[test]
fn a_writing_seat_writes_what_git_commits_need_and_never_hooks_or_config() {
    let g = GitDirs {
        common: PathBuf::from("/r/.git"),
        own: Some(PathBuf::from("/r/.git/worktrees/r-rook")),
    };
    let w = writable(Path::new("/r"), Role::Builder, Some(&g));
    for want in [
        "/r/.git/objects",
        "/r/.git/refs",
        "/r/.git/logs",
        "/r/.git/packed-refs.lock",
        "/r/.git/worktrees/r-rook",
    ] {
        assert!(w.contains(&PathBuf::from(want)), "{want} in {w:?}");
    }
    assert!(
        !w.contains(&g.common),
        "never the git directory whole: {w:?}"
    );
    for w in &w {
        assert!(
            !w.starts_with("/r/.git/hooks")
                && !w.starts_with("/r/.git/config")
                && !w.starts_with("/r/.git/info")
                && !w.starts_with("/r/.git/worktrees/r-heron"),
            "{} is writable",
            w.display()
        );
    }
    let d = git_protected(&g, Path::new("/r-rook"));
    for want in [
        "/r/.git/hooks",
        "/r/.git/config",
        "/r/.git/info",
        "/r/.git/worktrees/r-rook/commondir",
        "/r/.git/worktrees/r-rook/gitdir",
        "/r/.git/worktrees/r-rook/config.worktree",
        "/r-rook/.git",
    ] {
        assert!(d.contains(&PathBuf::from(want)), "{want} in {d:?}");
    }
    // In the main checkout there is no worktree entry to protect.
    let main = GitDirs {
        common: PathBuf::from("/r/.git"),
        own: None,
    };
    assert!(!git_protected(&main, Path::new("/r")).contains(&PathBuf::from("/r/.git")));
}
