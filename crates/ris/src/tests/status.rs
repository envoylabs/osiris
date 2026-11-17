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

/// A seat's line in the summary: running and on what, or why it
/// stopped with the command that brings it back and what comes before
/// that command. A running seat and an attended one name no command.
#[test]
fn a_summary_line_says_what_brings_a_stopped_seat_back() {
    use crate::schema::SummarySeat;
    let seat = |stopped: Option<&'static str>| SummarySeat {
        seat: "rook".into(),
        role: Role::Builder,
        running: false,
        claim: None,
        activity: None,
        stopped: stopped.map(|reason| Stopped {
            reason,
            resets: None,
        }),
        attended: false,
        next: Some("ris up rook".into()),
    };
    for (reason, line) in [
        (
            "usage_limit",
            "stopped: usage limit, idle; `ris up rook` once the limit resets",
        ),
        (
            "setup_fault",
            "stopped: setup fault, idle; `ris status` says what is missing: \
             allow it in [runner], then `ris up rook`",
        ),
        (
            "paused",
            "stopped: paused by the person, idle; `ris up rook`",
        ),
        (
            "nothing_left",
            "stopped: nothing left to claim, idle; `ris up rook` once there is more for it",
        ),
        (
            "ended",
            "stopped: ended, idle; `ris logs rook` shows how it ended, `ris up rook` starts it again",
        ),
    ] {
        assert_eq!(
            summary_line(&seat(Some(reason))),
            format!("{:<10} {line}", "rook")
        );
    }

    // Running: what it holds, no command; a quiet one is marked.
    let mut s = seat(None);
    s.running = true;
    s.next = None;
    s.claim = Some("I-7".into());
    assert_eq!(summary_line(&s), "rook       running, on I-7");
    s.activity = Some(Activity {
        last: None,
        quiet_seconds: 900,
        stuck: true,
    });
    assert_eq!(
        summary_line(&s),
        "rook       running, on I-7; possibly stuck: quiet 15m, `ris logs rook` shows its output"
    );

    // Attended and not running: the person's to run, so no command.
    let mut s = seat(None);
    s.attended = true;
    s.next = None;
    assert_eq!(summary_line(&s), "rook       attended, idle");
}
