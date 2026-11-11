//! `ris decide` with no decision named: the person's queue of open
//! substantive decisions, walked one per screen. Each screen is the
//! decision's case (what `ris eli5` shows) and then a prompt: a single
//! key picks an option, a line gives the rationale or Enter skips it,
//! and the walk moves to the next. Each decision is taken as it is
//! picked, so quitting keeps what was decided.
//!
//! In a terminal, keys are read in raw mode, so a key needs no Enter.
//! With stdin not a terminal (a script, a test), each prompt reads one
//! line and its first character is the key.

use std::io::{self, BufRead, IsTerminal, Write};

use ris_core::command::{Choice, Command};
use ris_core::ids::{Actor, DecisionId};
use ris_core::primitives::{DecisionClass, DecisionState};
use serde_json::json;

use crate::project::{CliError, Loaded, Project};
use crate::render::{self, Output};

/// The person's queue: every open substantive decision, oldest first,
/// whatever phase raised it.
pub fn queue(l: &Loaded) -> Vec<DecisionId> {
    l.state
        .decisions
        .values()
        .filter(|d| d.class == DecisionClass::Substantive && d.state == DecisionState::Open)
        .map(|d| d.id)
        .collect()
}

/// `ris decide --json` with no decision: the queue, each entry the
/// document `ris eli5 --json` gives for it, for an editor mode or a GUI
/// to show and answer with `ris decide D-n --choose k`.
pub fn queue_output(l: &Loaded) -> Output {
    let ids = queue(l);
    let entries: Vec<_> = ids
        .iter()
        .map(|id| render::eli5_case(l, &l.state.decisions[id]).json)
        .collect();
    let text = if ids.is_empty() {
        "no open substantive decisions".to_owned()
    } else {
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    Output::new(text, json!({ "queue": entries }))
}

/// What a key does at the prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Pick(usize),
    Skip,
    Quit,
}

/// A key read at the prompt, for a decision with `options` options:
/// a digit naming one picks it, `s` moves on without deciding, `q`
/// quits. Anything else is not an answer and is read past.
pub fn key(c: char, options: usize) -> Option<Key> {
    match c {
        'q' | 'Q' => Some(Key::Quit),
        's' | 'S' => Some(Key::Skip),
        _ => c
            .to_digit(10)
            .map(|n| n as usize)
            .filter(|n| *n < options)
            .map(Key::Pick),
    }
}

/// Where the walk's keys and rationale come from.
pub trait Input {
    /// The next key; `None` when input ends or the person interrupts.
    fn key(&mut self) -> io::Result<Option<char>>;
    /// A line of text, without its newline; empty when input ends.
    fn line(&mut self) -> io::Result<String>;
    /// Whether each decision gets a cleared screen.
    fn clears(&self) -> bool;
}

/// Lines from a reader, as stdin is when it is not a terminal: each
/// prompt takes one line, and a key is its first character.
pub struct Lines<R>(pub R);

impl<R: BufRead> Input for Lines<R> {
    fn key(&mut self) -> io::Result<Option<char>> {
        let mut s = String::new();
        if self.0.read_line(&mut s)? == 0 {
            return Ok(None);
        }
        Ok(Some(s.trim().chars().next().unwrap_or(' ')))
    }

    fn line(&mut self) -> io::Result<String> {
        let mut s = String::new();
        self.0.read_line(&mut s)?;
        Ok(s.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn clears(&self) -> bool {
        false
    }
}

/// The terminal: a key is read in raw mode, so it needs no Enter; the
/// rationale is read as an ordinary line.
pub struct Terminal;

/// Raw mode for as long as it lives, so every way out restores the
/// terminal.
struct Raw;

impl Raw {
    fn on() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

impl Input for Terminal {
    fn key(&mut self) -> io::Result<Option<char>> {
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
        let _raw = Raw::on()?;
        loop {
            let Event::Key(k) = read()? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Char('c' | 'd') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                KeyCode::Esc => return Ok(None),
                KeyCode::Char(c) => return Ok(Some(c)),
                _ => {}
            }
        }
    }

    fn line(&mut self) -> io::Result<String> {
        Lines(io::stdin().lock()).line()
    }

    fn clears(&self) -> bool {
        true
    }
}

/// Walk the queue with input from the terminal, or from stdin's lines
/// when it is not one.
pub fn walk_stdin(p: &mut Project, actor: &Actor) -> Result<Output, CliError> {
    let mut out = io::stdout();
    if io::stdin().is_terminal() {
        walk(p, actor, &mut Terminal, &mut out)
    } else {
        walk(p, actor, &mut Lines(io::stdin().lock()), &mut out)
    }
}

/// Walk the queue: for each decision still open when its turn comes,
/// its case, then the prompt; a pick is taken at once with the
/// rationale given, which may be empty. Returns what was taken and what
/// is still open.
pub fn walk(
    p: &mut Project,
    actor: &Actor,
    input: &mut dyn Input,
    out: &mut dyn Write,
) -> Result<Output, CliError> {
    if *actor != Actor::Person {
        return Err(CliError::Other(
            "the queue of substantive decisions is the person's to take".into(),
        ));
    }
    let ids = queue(&p.load()?);
    if ids.is_empty() {
        return Ok(Output::new(
            "no open substantive decisions",
            json!({ "taken": [], "open": [] }),
        ));
    }
    let mut taken = Vec::new();
    let mut quit = false;
    for id in &ids {
        if quit {
            break;
        }
        let l = p.load()?;
        let d = &l.state.decisions[id];
        // Taken by another command since the walk began.
        if d.state != DecisionState::Open {
            continue;
        }
        let options = d.options;
        if input.clears() {
            write!(out, "\x1b[2J\x1b[H")?;
        } else if !taken.is_empty() || id != &ids[0] {
            writeln!(out)?;
        }
        write!(out, "{}", render::eli5_case(&l, d).text)?;
        write!(
            out,
            "\n{id}: pick 0-{}, s to skip, q to quit: ",
            options.saturating_sub(1).min(9)
        )?;
        out.flush()?;
        let key = loop {
            match input.key()? {
                None => break Key::Quit,
                Some(c) => {
                    if let Some(k) = key(c, options) {
                        break k;
                    }
                }
            }
        };
        match key {
            Key::Quit => {
                writeln!(out)?;
                quit = true;
            }
            Key::Skip => writeln!(out, "s")?,
            Key::Pick(n) => {
                write!(out, "{n}\nrationale (Enter skips): ")?;
                out.flush()?;
                let rationale = input.line()?;
                let rationale = p.text("rationale", &rationale)?;
                p.run(&Command::Decide {
                    decision: *id,
                    by: actor.clone(),
                    chosen: Choice::Option(n),
                    rationale,
                })?;
                writeln!(out, "{id} taken: option {n}")?;
                taken.push(*id);
            }
        }
    }
    let open = queue(&p.load()?);
    let list = |ids: &[DecisionId]| {
        ids.iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut text = format!(
        "\ntaken: {}",
        if taken.is_empty() {
            "none".to_owned()
        } else {
            list(&taken)
        }
    );
    if !open.is_empty() {
        text.push_str(&format!(
            "\nstill open: {}\n  next: `ris decide` again",
            list(&open)
        ));
    }
    Ok(Output::new(
        text,
        json!({
            "taken": taken.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
            "open": open.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digit_picks_only_an_option_that_exists() {
        assert_eq!(key('0', 2), Some(Key::Pick(0)));
        assert_eq!(key('1', 2), Some(Key::Pick(1)));
        assert_eq!(key('2', 2), None);
        assert_eq!(key('s', 2), Some(Key::Skip));
        assert_eq!(key('q', 2), Some(Key::Quit));
        assert_eq!(key('x', 2), None);
        assert_eq!(key('\n', 2), None);
    }

    #[test]
    fn lines_give_a_key_per_line_and_a_rationale_line() {
        let mut input = Lines("1\nbecause it is cheap\n\n".as_bytes());
        assert_eq!(input.key().unwrap(), Some('1'));
        assert_eq!(input.line().unwrap(), "because it is cheap");
        assert_eq!(input.key().unwrap(), Some(' '));
        assert_eq!(input.key().unwrap(), None);
        assert_eq!(input.line().unwrap(), "");
    }
}
