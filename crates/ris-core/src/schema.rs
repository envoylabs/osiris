//! JSON Schema for the types that serialise themselves by hand, behind
//! the `schema` feature. Every other serialised type derives its
//! schema beside its `Serialize`, so the two cannot drift; these few
//! write a string in a fixed form, and say that form here.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::event::Salt;
use crate::ids::{
    ActionId, Actor, ActorRepr, ClaimId, DecisionId, EvidenceId, Hash, ItemId, MessageId, PhaseId,
};

/// A string type with a pattern, named `$name` in the schema.
macro_rules! string_schema {
    ($type:ty, $name:literal, $pattern:literal, $description:literal) => {
        impl JsonSchema for $type {
            fn schema_name() -> Cow<'static, str> {
                $name.into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": $pattern,
                    "description": $description,
                })
            }
        }
    };
}

string_schema!(
    PhaseId,
    "PhaseId",
    "^P-[0-9]+$",
    "A phase's id, such as P-3."
);
string_schema!(ItemId, "ItemId", "^I-[0-9]+$", "An item's id, such as I-7.");
string_schema!(
    DecisionId,
    "DecisionId",
    "^D-[0-9]+$",
    "A decision's id, such as D-4."
);
string_schema!(
    ClaimId,
    "ClaimId",
    "^C-[0-9]+$",
    "A claim's id, such as C-1."
);
string_schema!(
    MessageId,
    "MessageId",
    "^M-[0-9]+$",
    "A message's id, such as M-2."
);
string_schema!(
    ActionId,
    "ActionId",
    "^A-[0-9]+$",
    "A person action's id, such as A-2."
);
string_schema!(
    Hash,
    "Hash",
    "^[0-9a-f]{64}$",
    "A SHA-256 hash: 64 lowercase hex characters."
);
string_schema!(
    EvidenceId,
    "EvidenceId",
    "^E-[0-9a-f]{64}$",
    "A piece of evidence, addressed by the hash of its content."
);
string_schema!(
    Salt,
    "Salt",
    "^[0-9a-f]{32}$",
    "An event's salt: 32 lowercase hex characters."
);

/// An actor is written as its typed form, `{"kind": "person"}` or
/// `{"kind": "agent", "seat": ...}`.
impl JsonSchema for Actor {
    fn schema_name() -> Cow<'static, str> {
        "Actor".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ActorRepr::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
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
}
