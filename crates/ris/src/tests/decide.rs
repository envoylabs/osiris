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
