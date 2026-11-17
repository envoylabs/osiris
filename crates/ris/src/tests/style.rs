use super::*;

#[test]
fn a_seats_colour_comes_from_its_name_alone() {
    // The same name gives the same colour every time it is asked.
    for name in ["heron", "rook", "wren", "kestrel", "kite"] {
        assert_eq!(seat_colour(name), seat_colour(name), "{name}");
    }
    // Pinned, so a change to the hash or the palette, which would give
    // every seat a different colour from the one it had, fails here.
    let got: Vec<AnsiColor> = ["heron", "rook", "wren"]
        .iter()
        .map(|n| seat_colour(n))
        .collect();
    assert_eq!(
        got,
        [AnsiColor::BrightBlue, AnsiColor::Cyan, AnsiColor::Green],
        "heron, rook, wren"
    );
}

#[test]
fn the_template_seats_take_different_colours() {
    // The seats `ris init` writes are heron, rook and wren; a project
    // that keeps them should see three colours.
    let got: Vec<(&str, AnsiColor)> = ["heron", "rook", "wren"]
        .iter()
        .map(|n| (*n, seat_colour(n)))
        .collect();
    let mut distinct: Vec<AnsiColor> = got.iter().map(|(_, c)| *c).collect();
    distinct.sort_by_key(|c| *c as u8);
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "{got:?}");
}

#[test]
fn no_seat_takes_red() {
    assert!(!SEAT_COLOURS.contains(&AnsiColor::Red));
    assert!(!SEAT_COLOURS.contains(&AnsiColor::BrightRed));
}

#[test]
fn a_coloured_prefix_keeps_the_seats_name_and_plain_output_is_untouched() {
    let prefix = "rook  |";
    assert_eq!(seat_if(false, "rook", prefix), prefix);
    let coloured = seat_if(true, "rook", prefix);
    assert!(coloured.starts_with('\x1b'), "{coloured:?}");
    assert!(coloured.contains(prefix), "{coloured:?}");
    assert!(coloured.ends_with("\x1b[0m"), "{coloured:?}");
}
