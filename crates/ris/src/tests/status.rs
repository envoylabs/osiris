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
