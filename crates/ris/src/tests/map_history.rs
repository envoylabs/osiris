//! This repository's own history, for the tests that run the map over
//! real data: a closed phase's change set, read from git as `ris map`
//! reads it, with no store.
//!
//! The history is the checkout's own, so such a test holds wherever the
//! repository is cloned whole. A shallow clone or an exported tree has
//! no closing merge to read: there [`closed_phase`] gives `None` and
//! the test is passed over. In a whole clone a phase that is not found
//! is a failure.

use super::*;

/// Git's answer in this repository, trimmed, or `None` when git refuses.
fn git_here(args: &[&str]) -> Option<String> {
    git_line(Path::new(env!("CARGO_MANIFEST_DIR")), args).ok()
}

/// The map of the latest closed phase whose closing merge's subject
/// starts with `subject` (`chore: close P-12: ` for one phase,
/// `chore: close P-` for the last one closed), under the default
/// configuration and with no judgement.
pub(super) fn closed_phase(subject: &str) -> Option<crate::schema::Map> {
    closed_phase_where(subject, &|_| true)
}

/// [`closed_phase`], of the latest such phase whose map `holds` what
/// the test needs: the closing merges are walked back from the newest.
/// What a phase changed is not the test's to assume (one may close with
/// documents alone), so when no phase holds it the test is passed over
/// with that said, as it is in a checkout without the history.
pub(super) fn closed_phase_where(
    subject: &str,
    holds: &dyn Fn(&crate::schema::Map) -> bool,
) -> Option<crate::schema::Map> {
    let merges = git_here(&[
        "log",
        "--first-parent",
        "--merges",
        "--format=%H %s",
        "HEAD",
    ]);
    // Each closing merge with its first parent, the newest first.
    let closings: Vec<(String, String)> = merges
        .as_deref()
        .unwrap_or("")
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(_, s)| s.starts_with(subject))
        .filter_map(|(tip, _)| {
            let base = git_here(&["rev-parse", "--verify", "--quiet", &format!("{tip}^1")])?;
            Some((tip.to_owned(), base))
        })
        .collect();
    if closings.is_empty() {
        let whole = git_here(&["rev-parse", "--is-shallow-repository"]).as_deref() == Some("false");
        assert!(
            !whole,
            "a whole clone of this repository holds the closing merge `{subject}` and its first parent"
        );
        eprintln!("passed over: this checkout does not hold the history of `{subject}`");
        return None;
    }
    // The map's paths are the repository's, so git is asked at its root.
    let root = git_here(&["rev-parse", "--show-toplevel"]).expect("the checkout's root");
    for (tip, base) in closings {
        let range = Range {
            phase: None,
            base: Some(base),
            tip,
            tip_ref: "HEAD".to_owned(),
        };
        let map = build_in(
            Path::new(&root),
            &crate::config::MapConfig::default(),
            &BTreeMap::new(),
            &range,
        )
        .unwrap_or_else(|e| panic!("the phase's map is read from git: {e}"));
        if holds(&map) {
            return Some(map);
        }
    }
    eprintln!("passed over: no closed phase `{subject}` holds what the test needs");
    None
}
