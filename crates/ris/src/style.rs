//! Colour, when it helps and only then.
//!
//! Styling is on when stdout is a terminal, `NO_COLOR` is unset and the
//! output is text rather than JSON; otherwise every helper returns its
//! input unchanged, so transcripts and pipes stay plain. The choice is
//! made once by the binary and read everywhere through [`on`].

use std::io::IsTerminal;
use std::sync::OnceLock;

use anstyle::{AnsiColor, Style};

static ENABLED: OnceLock<bool> = OnceLock::new();

/// Decide once. `json` forces plain output.
pub fn init(json: bool) {
    let on = !json && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    let _ = ENABLED.set(on);
}

/// Whether styling is on. Plain until [`init`] has run.
pub fn on() -> bool {
    ENABLED.get().copied().unwrap_or(false)
}

fn paint(style: Style, s: &str) -> String {
    if on() {
        format!("{style}{s}{style:#}")
    } else {
        s.to_owned()
    }
}

/// A section header: `== report ==`.
pub fn header(s: &str) -> String {
    paint(
        Style::new().bold().fg_color(Some(AnsiColor::Cyan.into())),
        s,
    )
}

/// A sub-heading inside a section: `substantive decisions:`.
pub fn heading(s: &str) -> String {
    paint(Style::new().bold(), s)
}

/// An item's state word, coloured by what it means for the person.
pub fn state(s: &str) -> String {
    let style = match s {
        "ready" => Style::new().fg_color(Some(AnsiColor::Green.into())),
        "claimed" => Style::new().fg_color(Some(AnsiColor::Yellow.into())),
        "blocked" => Style::new().fg_color(Some(AnsiColor::Red.into())),
        "done" => Style::new().fg_color(Some(AnsiColor::Blue.into())),
        "merged" => Style::new().dimmed(),
        _ => Style::new(),
    };
    paint(style, s)
}

/// A check that passed.
pub fn passed(s: &str) -> String {
    paint(Style::new().fg_color(Some(AnsiColor::Green.into())), s)
}

/// A check that failed, or a refusal.
pub fn failed(s: &str) -> String {
    paint(Style::new().bold().fg_color(Some(AnsiColor::Red.into())), s)
}

/// The recommendation mark on an option.
pub fn recommended(s: &str) -> String {
    paint(
        Style::new().bold().fg_color(Some(AnsiColor::Green.into())),
        s,
    )
}

/// The mark on an answer given outside the options.
pub fn other(s: &str) -> String {
    paint(
        Style::new()
            .bold()
            .fg_color(Some(AnsiColor::Magenta.into())),
        s,
    )
}

/// De-emphasised text: hashes, footers.
pub fn dim(s: &str) -> String {
    paint(Style::new().dimmed(), s)
}
