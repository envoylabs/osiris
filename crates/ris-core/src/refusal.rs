//! Refusals: the typed answer when a command cannot be applied.
//!
//! A refusal is part of the contract, not an error path: an agent reads
//! it and knows what to do next. Variants are added as commands are.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::text::Category;

/// Why a command was not applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// A free-text field failed the confidentiality scan. The match is
    /// never echoed: only the field and the category are named.
    Confidential {
        /// The field that was refused, as the caller named it.
        field: String,
        /// What kind of thing the scan found.
        category: Category,
    },
}

impl Refusal {
    /// What the actor should do about it, in one line.
    pub fn next_step(&self) -> String {
        match self {
            Self::Confidential { field, category } => format!(
                "remove the {} from `{field}` and try again; \
                 harness state and commits never carry one",
                category.noun()
            ),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confidential { field, category } => {
                write!(f, "refused: `{field}` contains {}", category.description())
            }
        }
    }
}

impl std::error::Error for Refusal {}
