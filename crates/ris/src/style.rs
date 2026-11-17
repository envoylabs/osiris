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
    paint_if(on(), style, s)
}

fn paint_if(on: bool, style: Style, s: &str) -> String {
    if on {
        format!("{style}{s}{style:#}")
    } else {
        s.to_owned()
    }
}

/// The colours a seat may take: few enough to tell apart at a glance
/// on most terminal themes. Red is left out, since elsewhere it means a
/// failure or a refusal.
const SEAT_COLOURS: [AnsiColor; 6] = [
    AnsiColor::Cyan,
    AnsiColor::Green,
    AnsiColor::Yellow,
    AnsiColor::Blue,
    AnsiColor::Magenta,
    AnsiColor::BrightBlue,
];

/// A seat's colour, from its name alone, so it is the same on every
/// run and every machine: a 32-bit FNV-1a hash of the name picks from
/// [`SEAT_COLOURS`]. The standard library's hasher is not used because
/// it does not promise the same output between Rust releases. Two
/// seats may share a colour; the seat's name is always printed beside
/// it, so nothing depends on the colour.
pub fn seat_colour(seat: &str) -> AnsiColor {
    let mut h: u32 = 0x811c_9dc5;
    for b in seat.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    SEAT_COLOURS[h as usize % SEAT_COLOURS.len()]
}

/// `s` in `seat`'s colour: the prefix `ris logs` puts on each line
/// when it shows more than one seat.
pub fn seat(seat: &str, s: &str) -> String {
    seat_if(on(), seat, s)
}

/// [`seat`] with the choice made by the caller.
pub fn seat_if(on: bool, seat: &str, s: &str) -> String {
    paint_if(on, Style::new().fg_color(Some(seat_colour(seat).into())), s)
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

#[cfg(test)]
#[path = "tests/style.rs"]
mod tests;
