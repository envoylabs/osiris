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
#[path = "tests/schema.rs"]
mod tests;
