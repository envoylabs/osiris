//! `ris decide` with no decision named: the person's queue of open
//! substantive decisions, walked one per screen. Each screen is the
//! decision's case (what `ris eli5` shows) and then a prompt: a single
//! key picks an option, or `o` takes an answer that is none of them,
//! typed as a line; a line gives the rationale or Enter skips it, and
//! the walk moves to the next. Each decision is taken as it is
//! answered, so quitting keeps what was decided.
//!
//! In a terminal, keys are read in raw mode, so a key needs no Enter.
//! With stdin not a terminal (a script, a test), each prompt reads one
//! line and its first character is the key.

use std::io::{self, BufRead, IsTerminal, Write};

use crate::project::{CliError, Loaded, Project};
use crate::render::{self, Output};
use crate::schema;
use ris_core::command::{Choice, Command};
use ris_core::ids::{Actor, DecisionId};
use ris_core::primitives::{DecisionClass, DecisionState};

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
    let queue = ids
        .iter()
        .map(|id| render::eli5_case(l, &l.state.decisions[id]).1)
        .collect();
    let text = if ids.is_empty() {
        "no open substantive decisions".to_owned()
    } else {
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    Output::of(text, &schema::Decide::Queue { queue })
}

/// What a key does at the prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Pick(usize),
    /// None of the options: the person's own answer, asked for as text.
    Own,
    Skip,
    Quit,
}

/// A key read at the prompt, for a decision with `options` options:
/// a digit naming one picks it, `o` answers with none of them, `s`
/// moves on without deciding, `q` quits. Anything else is not an answer
/// and is read past.
pub fn key(c: char, options: usize) -> Option<Key> {
    match c {
        'q' | 'Q' => Some(Key::Quit),
        's' | 'S' => Some(Key::Skip),
        'o' | 'O' => Some(Key::Own),
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

/// The prompt under a decision's case. It names every way to answer on
/// every screen, the person's own answer among them, since the options
/// offered are never the only answers.
pub fn prompt(id: DecisionId, options: usize) -> String {
    format!(
        "{id}: pick 0-{}, o for none of these (your own answer), s to skip, q to quit: ",
        options.saturating_sub(1).min(9)
    )
}

/// Walk the queue: for each decision still open when its turn comes,
/// its case, then the prompt; a pick is taken at once with the
/// rationale given, which may be empty. `o` asks for the person's own
/// answer as a line and records it as `ris decide D-n --other` does; an
/// empty line there returns to the prompt, deciding nothing. Returns
/// what was taken and what is still open.
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
        return Ok(Output::of(
            "no open substantive decisions",
            &schema::Decide::Walk {
                taken: Vec::new(),
                open: Vec::new(),
            },
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
        write!(out, "{}", render::eli5_case(&l, d).0)?;
        writeln!(out)?;
        // Until the decision is answered, skipped or the walk quits: an
        // own answer left empty comes back here.
        loop {
            write!(out, "{}", prompt(*id, options))?;
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
            let (chosen, picked) = match key {
                Key::Quit => {
                    writeln!(out)?;
                    quit = true;
                    break;
                }
                Key::Skip => {
                    writeln!(out, "s")?;
                    break;
                }
                Key::Pick(n) => {
                    writeln!(out, "{n}")?;
                    (Choice::Option(n), format!("option {n}"))
                }
                Key::Own => {
                    write!(out, "o\nyour own answer (Enter goes back): ")?;
                    out.flush()?;
                    let answer = input.line()?;
                    if answer.trim().is_empty() {
                        continue;
                    }
                    (
                        Choice::Other(p.text("other", answer.trim())?),
                        "your own answer, recorded as a new option".to_owned(),
                    )
                }
            };
            write!(out, "rationale (Enter skips): ")?;
            out.flush()?;
            let rationale = input.line()?;
            let rationale = p.text("rationale", &rationale)?;
            p.run(&Command::Decide {
                decision: *id,
                by: actor.clone(),
                chosen,
                rationale,
            })?;
            writeln!(out, "{id} taken: {picked}")?;
            taken.push(*id);
            break;
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
    Ok(Output::of(text, &schema::Decide::Walk { taken, open }))
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
        assert_eq!(key('o', 2), Some(Key::Own));
        assert_eq!(key('O', 0), Some(Key::Own));
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
