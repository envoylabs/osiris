use schemars::generate::SchemaSettings;

use crate::event::{Event, Structure};
use crate::ids::{Actor, AgentName, Hash, ItemId};
use crate::text::Text;

/// The patterns written here against what the types really write.
#[test]
fn the_hand_written_patterns_match_what_is_serialised() {
    let mut generator = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    let schema = generator.root_schema_for::<Event>().to_value();
    let defs = &schema["$defs"];
    let matches = |name: &str, value: serde_json::Value| {
        let pattern = defs[name]["pattern"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no pattern"));
        let text = value.as_str().expect("a string").to_owned();
        assert!(
            regex::Regex::new(pattern).unwrap().is_match(&text),
            "{name}: {text} does not match {pattern}"
        );
    };
    let salt = crate::event::Salt([7; 16]);
    let event = Event::new(
        1,
        Hash::ZERO,
        0,
        Structure::ItemClaimed {
            item: ItemId(12),
            agent: AgentName("heron".into()),
        },
        std::collections::BTreeMap::<String, Text>::new(),
        salt,
    );
    let json = serde_json::to_value(&event).unwrap();
    matches("Hash", json["hash"].clone());
    matches("Hash", json["prev"].clone());
    matches("Salt", json["salt"].clone());
    matches("ItemId", json["structure"]["item"].clone());
    matches(
        "EvidenceId",
        serde_json::to_value(crate::ids::EvidenceId(Hash::ZERO)).unwrap(),
    );
    // An actor's schema is its typed form's: both of its kinds.
    let actor = generator.root_schema_for::<Actor>().to_value().to_string();
    for part in ["\"person\"", "\"agent\"", "\"seat\""] {
        assert!(actor.contains(part), "{part} in {actor}");
    }
}
