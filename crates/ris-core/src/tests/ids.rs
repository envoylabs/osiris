use super::*;

#[test]
fn ids_round_trip_through_their_display_form() {
    let id = ItemId(7);
    assert_eq!(id.to_string(), "I-7");
    assert_eq!("I-7".parse::<ItemId>(), Ok(id));
    assert_eq!("D-7".parse::<DecisionId>(), Ok(DecisionId(7)));
}

#[test]
fn ids_of_the_wrong_kind_are_refused() {
    assert_eq!(
        "D-7".parse::<ItemId>(),
        Err(IdParseError::WrongPrefix { expected: "I" })
    );
    assert_eq!("I-x".parse::<ItemId>(), Err(IdParseError::NotANumber));
    assert_eq!(
        "D7".parse::<ItemId>(),
        Err(IdParseError::WrongPrefix { expected: "I" })
    );
    for bad in ["I", "I-", "I--7", "I 7", "I-7x", "7", "I+7", "I-+7", "x7"] {
        assert!(bad.parse::<ItemId>().is_err(), "{bad:?}");
    }
}

#[test]
fn the_hyphen_is_optional_on_input_and_always_shown() {
    assert_eq!("I7".parse::<ItemId>(), Ok(ItemId(7)));
    assert_eq!("D5".parse::<DecisionId>(), Ok(DecisionId(5)));
    assert_eq!("P12".parse::<PhaseId>(), Ok(PhaseId(12)));
    assert_eq!("M3".parse::<MessageId>(), Ok(MessageId(3)));
    assert_eq!("a-4".parse::<ActionId>().unwrap().to_string(), "A-4");
    assert_eq!("D5".parse::<DecisionId>().unwrap().to_string(), "D-5");
    assert_eq!("d5".parse::<DecisionId>(), Ok(DecisionId(5)));
    assert_eq!("i-7".parse::<ItemId>(), Ok(ItemId(7)));
    assert_eq!("d5".parse::<DecisionId>().unwrap().to_string(), "D-5");
}

#[test]
fn hashes_display_as_lowercase_hex() {
    let mut bytes = [0u8; 32];
    bytes[0] = 0xab;
    bytes[31] = 0x01;
    let hex = Hash(bytes).to_hex();
    assert_eq!(hex.len(), 64);
    assert!(hex.starts_with("ab00"));
    assert!(hex.ends_with("01"));
}
