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
    let merges = git_here(&[
        "log",
        "--first-parent",
        "--merges",
        "--format=%H %s",
        "HEAD",
    ]);
    let closing = merges.as_deref().and_then(|log| {
        log.lines()
            .filter_map(|line| line.split_once(' '))
            .find(|(_, s)| s.starts_with(subject))
            .map(|(sha, _)| sha.to_owned())
    });
    let base = closing
        .as_deref()
        .and_then(|tip| git_here(&["rev-parse", "--verify", "--quiet", &format!("{tip}^1")]));
    let (Some(tip), Some(base)) = (closing, base) else {
        let whole = git_here(&["rev-parse", "--is-shallow-repository"]).as_deref() == Some("false");
        assert!(
            !whole,
            "a whole clone of this repository holds the closing merge `{subject}` and its first parent"
        );
        eprintln!("passed over: this checkout does not hold the history of `{subject}`");
        return None;
    };
    let range = Range {
        phase: None,
        base: Some(base),
        tip,
        tip_ref: "HEAD".to_owned(),
    };
    // The map's paths are the repository's, so git is asked at its root.
    let root = git_here(&["rev-parse", "--show-toplevel"]).expect("the checkout's root");
    Some(
        build_in(
            Path::new(&root),
            &crate::config::MapConfig::default(),
            &BTreeMap::new(),
            &range,
        )
        .unwrap_or_else(|e| panic!("the phase's map is read from git: {e}")),
    )
}
