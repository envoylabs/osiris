//! Identifiers.
//!
//! Short, typed and assigned in sequence per kind, so a person can type
//! them: `I-1`, `D-1`, `C-1`. Evidence is addressed by the hash of its
//! content. One store per project means no namespace is needed.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! sequential_id {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub u32);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{}", $prefix, self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdParseError;

            /// The hyphen is for reading, so input may leave it out:
            /// `D-5` and `D5` are the same id. Output always has it.
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let rest = s
                    .strip_prefix($prefix)
                    .map(|r| r.strip_prefix('-').unwrap_or(r))
                    .ok_or(IdParseError::WrongPrefix { expected: $prefix })?;
                rest.parse::<u32>()
                    .map(Self)
                    .map_err(|_| IdParseError::NotANumber)
            }
        }
    };
}

sequential_id!(PhaseId, "P", "A phase: a set of items with an exit review.");
sequential_id!(ItemId, "I", "An item: one unit of work and one commit.");
sequential_id!(
    DecisionId,
    "D",
    "A decision: a question with options, mechanical or substantive."
);
sequential_id!(
    ClaimId,
    "C",
    "A claim: a statement the harness holds, with provenance and a falsifier."
);
sequential_id!(
    MessageId,
    "M",
    "A message: a typed, targeted note between agents."
);

/// Why an id failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdParseError {
    /// The text did not start with the kind's prefix and a dash.
    WrongPrefix {
        /// The prefix that was expected.
        expected: &'static str,
    },
    /// The part after the dash was not a number.
    NotANumber,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPrefix { expected } => write!(f, "expected an id like {expected}-1"),
            Self::NotANumber => write!(f, "the part after the dash must be a number"),
        }
    }
}

impl std::error::Error for IdParseError {}

/// A check's name, as declared in the project's checks file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CheckId(pub String);

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A 32-byte hash, hex-encoded when displayed. The default is all
/// zeros: the `prev` of a log's first event.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The hex encoding, 64 characters.
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..12])
    }
}

/// Evidence is addressed by the hash of its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EvidenceId(pub Hash);

impl fmt::Display for EvidenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E-{}", self.0)
    }
}

/// A git commit, by its full hash as git prints it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitHash(pub String);

impl fmt::Display for CommitHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An agent's configured name: a durable pseudonym for a seat, never a
/// real person's name. The configuration that declares it is scanned
/// like any other free text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentName(pub String);

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who did something: the person, or an agent under its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Actor {
    /// The one person directing the work.
    Person,
    /// An agent, by its configured name.
    Agent(AgentName),
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Person => f.write_str("the person"),
            Self::Agent(name) => write!(f, "agent {name}"),
        }
    }
}

#[cfg(test)]
mod tests {
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
        for bad in ["I", "I-", "I--7", "I 7", "I-7x", "7", "i7"] {
            assert!(bad.parse::<ItemId>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_hyphen_is_optional_on_input_and_always_shown() {
        assert_eq!("I7".parse::<ItemId>(), Ok(ItemId(7)));
        assert_eq!("D5".parse::<DecisionId>(), Ok(DecisionId(5)));
        assert_eq!("P12".parse::<PhaseId>(), Ok(PhaseId(12)));
        assert_eq!("M3".parse::<MessageId>(), Ok(MessageId(3)));
        assert_eq!("D5".parse::<DecisionId>().unwrap().to_string(), "D-5");
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
}
