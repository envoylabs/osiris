//! `ris phase review`: the person's review step before closing a
//! phase. It shows the phase's state, walks the open decisions as
//! `ris decide` does, shows the change set ranked by risk (what
//! `ris map --list` prints), and records that the phase was reviewed at
//! the tip of its work. `ris phase close` refuses a tip that has not
//! been reviewed, unless told to close unreviewed.

use std::io::Write;

use crate::project::{CliError, Loaded, Project};
use crate::render::Output;
use crate::schema;
use ris_core::command::Command;
use ris_core::ids::{Actor, CommitHash, PhaseId};
use ris_core::refusal::Refusal;

/// The commit at the head of the open phase's work, as `ris map` reads
/// it: the phase's branch, or the checkout when there is none. `None`
/// when it cannot be read, as in a project that is not in git.
pub fn tip(p: &Project, l: &Loaded) -> Option<CommitHash> {
    let phase = l.state.open_phase()?.id;
    crate::map::range(p, l, Some(phase))
        .ok()
        .map(|r| CommitHash(r.tip))
}

/// Whether the open phase's latest review stands at `now`, the tip of
/// its work as [`tip`] reads it: what the summary says and the close
/// asks for, so the two agree.
pub fn stands(l: &Loaded, now: Option<&CommitHash>) -> bool {
    l.state
        .open_phase()
        .is_some_and(|p| unreviewed(l, p.id, now).is_none())
}

/// Why the open phase may not close unreviewed, or `None` when its
/// latest review stands at the tip of its work now.
pub fn unreviewed(l: &Loaded, phase: PhaseId, now: Option<&CommitHash>) -> Option<String> {
    let flag = "`ris phase close --unreviewed` closes it without one";
    match l.state.reviews.get(&phase) {
        Some(r) if r.tip.as_ref() == now => None,
        Some(r) => Some(format!(
            "refused: {phase} was reviewed at {}, and its work has moved since\n  \
             next: `ris phase review` reviews it again; {flag}",
            r.tip.as_ref().map_or("no commit", |c| short(&c.0))
        )),
        None => Some(format!(
            "refused: {phase} has not been reviewed: its change set is closed unread\n  \
             next: `ris phase review` shows its state, its open decisions and its change set, \
             and records the review; {flag}"
        )),
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// Run the review. `state` is the phase's state as `ris summary` gives
/// it, printed first; `out` is where the review writes as it goes.
/// With `walk` false (the `--json` form), the decisions are not walked:
/// those open are listed in the data.
pub fn run(
    p: &mut Project,
    actor: &Actor,
    state: &str,
    walk: bool,
    out: &mut dyn Write,
) -> Result<Output, CliError> {
    if *actor != Actor::Person {
        return Err(CliError::Refused(Refusal::ReviewIsForThePerson));
    }
    let l = p.load()?;
    let phase = l.state.open_phase().ok_or(Refusal::NoOpenPhase)?.id;
    if walk {
        writeln!(out, "{state}\n")?;
    }
    // The decisions, as `ris decide` walks them.
    let before = crate::decide::queue(&l);
    if walk && !before.is_empty() {
        writeln!(out, "the open decisions, one at a time:")?;
        let walked = crate::decide::walk_stdin_to(p, actor, out)?;
        writeln!(out, "{}\n", walked.text.trim())?;
    }
    let open = crate::decide::queue(&p.load()?);
    let taken = before.into_iter().filter(|d| !open.contains(d)).collect();
    // The change set, ranked by risk.
    let l = p.load()?;
    let (map, tip, list) = match crate::map::range(p, &l, Some(phase)) {
        Ok(range) => {
            let map = crate::map::build(p, &l, &range)?;
            // The one-step marks as a count: `ris map --list --steps`
            // names each, and the list's last lines say so.
            let list =
                crate::map::output(map.clone(), crate::map::Show::List { steps: false })?.text;
            (Some(Box::new(map)), Some(CommitHash(range.tip)), list)
        }
        Err(e) => (
            None,
            None,
            format!("no change set to show: {}", e.to_string().trim()),
        ),
    };
    p.run(&Command::PhaseReview {
        by: actor.clone(),
        tip: tip.clone(),
    })?;
    let at = tip
        .as_ref()
        .map_or(String::new(), |c| format!(" at {}", short(&c.0)));
    let mut text = format!("{list}\n\nreviewed {phase}{at}");
    if !open.is_empty() {
        text.push_str(&format!(
            "; {} substantive decision(s) still open: `ris decide` takes them",
            open.len()
        ));
    }
    text.push_str(
        "\n  next: `ris phase close`; a merge before then moves the phase on, \
         and the close asks for the review again",
    );
    Ok(Output::of(
        text,
        &schema::PhaseReviewed {
            phase,
            tip: tip.map(|c| c.0),
            taken,
            open,
            map,
        },
    ))
}
