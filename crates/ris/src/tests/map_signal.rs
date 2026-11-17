//! Whether the map's one-step marks carry signal, measured over a real
//! phase: this repository's P-12, read from its history.
//!
//! With every call resolved by its bare name, 53 of that phase's 103
//! changed functions were marked one step from an effect, and the list
//! spent 152 of its 220 lines on those marks: 23 on a method named
//! `get` matched to every call of any `get`, 27 on the command's
//! dispatcher, which calls everything, and 15 on the git helper.

use super::*;

/// What the marks of a phase's map come to.
#[derive(Debug)]
struct Measure {
    /// The changed functions, and those of them one step from an effect
    /// by a step that counts.
    functions: usize,
    one_step: usize,
    /// The list's lines, and those of them spent on one-step marks,
    /// folded and with each named.
    lines: usize,
    step_lines: usize,
    lines_unfolded: usize,
    step_lines_unfolded: usize,
    /// The steps by the function they reach: the count, the function
    /// with its file, and whether it is wide.
    targets: Vec<(usize, String, bool)>,
}

fn measure(map: &crate::schema::Map) -> Measure {
    let changed: Vec<&ChangedFunction> = map
        .nodes
        .iter()
        .flat_map(|n| n.functions.iter().flatten())
        .collect();
    let mut by_target: BTreeMap<(String, bool), usize> = BTreeMap::new();
    for s in changed.iter().flat_map(|f| &f.steps) {
        *by_target
            .entry((format!("{} ({})", s.function, s.path), s.wide))
            .or_default() += 1;
    }
    let mut targets: Vec<(usize, String, bool)> = by_target
        .into_iter()
        .map(|((target, wide), n)| (n, target, wide))
        .collect();
    targets.sort_by(|a, b| b.cmp(a));
    let list = |steps: bool| {
        let text = output(map.clone(), Show::List { steps }).unwrap().text;
        let on_steps = text
            .lines()
            .filter(|l| {
                l.contains(" triggers ")
                    || l.contains(" trigger ")
                    || l.contains(" feeds ")
                    || l.contains(" feed ")
                    || l.contains("one step from")
            })
            .count();
        (text.lines().count(), on_steps)
    };
    let (lines, step_lines) = list(false);
    let (lines_unfolded, step_lines_unfolded) = list(true);
    Measure {
        functions: changed.len(),
        one_step: changed
            .iter()
            .filter(|f| !f.wide && f.steps.iter().any(|s| !s.wide))
            .count(),
        lines,
        step_lines,
        lines_unfolded,
        step_lines_unfolded,
        targets,
    }
}

#[test]
fn the_one_step_marks_of_a_real_phase_carry_signal() {
    let Some(map) = super::history::closed_phase("chore: close P-12: ") else {
        return;
    };
    let m = measure(&map);
    // The steps that reach the functions `name` names: with a file after
    // it that one function, otherwise every function so named.
    let reaching = |name: &str| -> usize {
        m.targets
            .iter()
            .filter(|(_, target, _)| target == name || target.starts_with(&format!("{name} (")))
            .map(|(n, _, _)| n)
            .sum()
    };
    const DISPATCHER: &str = "run (crates/ris/src/main.rs)";
    // The measurement, printed for the item's evidence.
    println!(
        "P-12: {} of {} changed functions one step from an effect",
        m.one_step, m.functions
    );
    println!(
        "P-12: {} of the list's {} lines on one-step marks ({} of {} with --steps)",
        m.step_lines, m.lines, m.step_lines_unfolded, m.lines_unfolded
    );
    println!(
        "P-12: steps via a method named get: {}; via the dispatcher (run): {}, said on {} line(s); \
         via the git helper: {}",
        reaching("EvidenceStore::get"),
        reaching(DISPATCHER),
        map.nodes
            .iter()
            .filter(|n| n.functions.iter().flatten().any(|f| f
                .steps
                .iter()
                .any(|s| s.wide && s.path == "crates/ris/src/main.rs")))
            .count(),
        reaching("git"),
    );
    for (n, target, wide) in m.targets.iter().take(8) {
        println!("  {n:>3} via {target}{}", if *wide { ", wide" } else { "" });
    }

    // The phase is history, so these numbers move only when the map's
    // rules do. The bounds are what the rules must keep.
    assert_eq!(m.functions, 103, "the phase's changed functions");
    assert!(
        m.one_step * 4 <= m.functions,
        "at most a quarter of the changed functions are one step from an effect: {m:?}"
    );
    assert!(
        m.step_lines * 4 <= m.lines,
        "the folded list spends at most a quarter of its lines on one-step marks: {m:?}"
    );
    assert!(
        m.step_lines_unfolded * 2 <= m.lines_unfolded,
        "with each named, at most half: {m:?}"
    );
    // No call is matched to a method by its bare name.
    assert_eq!(reaching("EvidenceStore::get"), 0, "{m:?}");
    // The dispatcher is wide, so it is said once for a file.
    assert!(
        m.targets
            .iter()
            .any(|(_, target, wide)| target == DISPATCHER && *wide),
        "{m:?}"
    );
    assert!(
        m.targets
            .iter()
            .all(|(_, target, wide)| target == DISPATCHER || !*wide),
        "the dispatcher is the one wide function the phase's changes reach: {m:?}"
    );
    // The git helper is reached only by the functions that call it.
    assert!(reaching("git") <= 5, "{m:?}");
}
