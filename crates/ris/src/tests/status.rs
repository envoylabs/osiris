use super::*;

/// A running seat that is also paused says so on its own line, not
/// at the end of the activity line beneath it.
#[test]
fn a_paused_running_seat_says_paused_on_its_own_line() {
    let s = Seat {
        seat: "heron".into(),
        role: Role::Builder,
        worktree: None,
        branch: Some("heron".into()),
        claim: Some("I-3".into()),
        ahead: Some(0),
        behind: Some(0),
        uncommitted: Some(0),
        paused: true,
        running: true,
        activity: Some(Activity {
            last: Some("→ Bash ris done I-3".into()),
            quiet_seconds: 5,
            stuck: false,
        }),
        faults: vec!["cannot write the build cache".into()],
        stopped: None,
    };
    let text = seat_lines(&s, "../heron");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(
        lines[2].trim_start(),
        "setup fault: cannot write the build cache"
    );
    assert!(
        lines[0].starts_with("heron") && lines[0].ends_with("running  paused"),
        "{text}"
    );
    assert!(
        lines[1].trim_start().starts_with("last 5s ago: ") && !lines[1].contains("paused"),
        "{text}"
    );
}

#[test]
fn a_stop_s_reason_is_the_first_that_holds() {
    use crate::runner::RunEnd;
    let limit = || RunEnd::UsageLimit {
        resets: Some("3:20am (UTC)".into()),
    };
    let reason = |end, faulted, paused, holds| stopped(end, faulted, paused, holds).reason;
    // The limit is the run's own ending, whatever else holds.
    assert_eq!(reason(limit(), true, true, true), "usage_limit");
    assert_eq!(reason(RunEnd::Finished, true, true, false), "setup_fault");
    assert_eq!(reason(RunEnd::Error, false, true, true), "paused");
    assert_eq!(
        reason(RunEnd::Finished, false, false, false),
        "nothing_left"
    );
    // Finished while still holding an item, or cut off: ended.
    assert_eq!(reason(RunEnd::Finished, false, false, true), "ended");
    assert_eq!(reason(RunEnd::NoResult, false, false, false), "ended");
    assert_eq!(reason(RunEnd::Error, false, false, false), "ended");
    // Only the limit carries a reset time.
    assert_eq!(
        stopped(limit(), false, false, false).resets.as_deref(),
        Some("3:20am (UTC)")
    );
    assert_eq!(stopped(RunEnd::Finished, true, false, false).resets, None);
}

#[test]
fn a_stopped_seat_says_why_beside_it() {
    let mut s = Seat {
        seat: "rook".into(),
        role: Role::Builder,
        worktree: None,
        branch: Some("rook".into()),
        claim: None,
        ahead: Some(0),
        behind: Some(0),
        uncommitted: Some(0),
        paused: false,
        running: false,
        activity: None,
        faults: Vec::new(),
        stopped: Some(Stopped {
            reason: "usage_limit",
            resets: Some("3:20am (UTC)".into()),
        }),
    };
    let text = seat_lines(&s, "../rook");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(
        text.ends_with("idle  stopped: usage limit, resets 3:20am (UTC)"),
        "{text}"
    );
    for (reason, words) in [
        ("nothing_left", "nothing left to claim"),
        ("paused", "paused by the person"),
        ("setup_fault", "setup fault"),
        ("ended", "ended"),
    ] {
        s.stopped = Some(Stopped {
            reason,
            resets: None,
        });
        let text = seat_lines(&s, "../rook");
        assert!(text.ends_with(&format!("stopped: {words}")), "{text}");
    }
    s.stopped = None;
    assert!(!seat_lines(&s, "../rook").contains("stopped"));
}
