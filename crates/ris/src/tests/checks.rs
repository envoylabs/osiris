use super::*;

// No group has this id: it is above the largest pid Linux or macOS
// gives out, so a kill that did act on it would reach nothing.
fn no_group() -> rustix::process::Pid {
    rustix::process::Pid::from_raw(0x7fff_fff0).unwrap()
}

#[test]
fn a_group_that_is_not_there_is_not_a_fault() {
    assert_eq!(kill_group(no_group()), None);
}

#[test]
fn a_kill_that_is_refused_is_reported_with_the_group_and_the_reason() {
    // The error is given, not provoked: a kill the system refuses
    // needs a group owned by someone else, and none is safe to aim
    // at.
    let fault = fault_of(no_group(), Err(rustix::io::Errno::PERM)).expect("a fault");
    assert!(
        fault.starts_with("killing process group 2147483632 failed: "),
        "{fault}"
    );
    assert_eq!(fault_of(no_group(), Ok(())), None);
}

/// The whole group goes, not only the shell: a background child the
/// shell started, which would write a file if left to run, never
/// writes it.
#[test]
fn a_command_stopped_at_its_limit_takes_what_it_started_with_it() {
    let dir = std::env::temp_dir().join(format!("ris-checks-group-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let limit = std::time::Duration::from_millis(200);
    match run_limited(&dir, "(sleep 1; touch late) & wait", limit).unwrap() {
        Limited::TimedOut { kill_fault } => assert_eq!(kill_fault, None),
        Limited::Finished(..) => panic!("the command ended within 200ms"),
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        !dir.join("late").exists(),
        "the background child ran on after the group was killed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_command_stopped_at_its_limit_reports_no_fault_when_the_kill_works() {
    let dir = std::env::temp_dir();
    let limit = std::time::Duration::from_millis(200);
    match run_limited(&dir, "sleep 30", limit).unwrap() {
        Limited::TimedOut { kill_fault } => assert_eq!(kill_fault, None),
        Limited::Finished(..) => panic!("sleep 30 ended within 200ms"),
    }
    match run_limited(&dir, "echo hi", std::time::Duration::from_secs(10)).unwrap() {
        Limited::Finished(passed, out) => {
            assert!(passed);
            assert_eq!(out, b"hi\n");
        }
        Limited::TimedOut { .. } => panic!("echo ran past 10s"),
    }
}

#[test]
fn the_agent_trailer_names_the_author() {
    let msg = "feat: x\n\nItem: I-1\nAgent: heron\n";
    assert!(require_agent_trailer(msg, "heron", "abc").is_ok());
    assert!(require_agent_trailer(msg, "wren", "abc").is_err());
    assert!(require_agent_trailer("feat: x\n\nItem: I-1\n", "heron", "abc").is_err());
    assert!(require_agent_trailer("Agent: heron\nAgent: wren\n", "heron", "abc").is_err());
}

#[test]
fn trailers_are_read_from_message_lines() {
    let msg = "feat(core): the fold\n\nBody text.\n\nItem: I-7\nCo-Authored-By: someone\n";
    assert_eq!(item_trailers(msg), vec![Ok(ItemId(7))]);
    assert!(require_trailer(msg, ItemId(7), "abcdef123456").is_ok());
    assert!(require_trailer(msg, ItemId(8), "abcdef123456").is_err());
    assert!(require_trailer("no trailer\n", ItemId(7), "abc").is_err());
    assert!(require_trailer("Item: I-1\nItem: I-2\n", ItemId(1), "abc").is_err());
    assert!(require_trailer("Item: seven\n", ItemId(7), "abc").is_err());
}

#[test]
fn a_check_records_its_output_as_evidence() {
    let dir = std::env::temp_dir().join(format!("ris-checks-test-{}", std::process::id()));
    // Process ids are reused; start from an empty directory.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = EvidenceStore::open(dir.join("evidence")).unwrap();
    let ok = run_check(
        &dir,
        "ok",
        &CheckConfig {
            command: "echo fine".into(),
        },
        &store,
    )
    .unwrap();
    assert!(ok.passed);
    let blob = store.get(ok.evidence).unwrap().unwrap();
    let text = String::from_utf8(blob).unwrap();
    assert!(text.starts_with("check: ok\ncommand: echo fine\nexit: 0\n"));
    assert!(text.contains("fine"));
    let bad = run_check(
        &dir,
        "bad",
        &CheckConfig {
            command: "echo boom >&2; exit 3".into(),
        },
        &store,
    )
    .unwrap();
    assert!(!bad.passed);
    let text = String::from_utf8(store.get(bad.evidence).unwrap().unwrap()).unwrap();
    assert!(text.contains("exit: 3"));
    assert!(text.contains("--- stderr ---\nboom"));
}
