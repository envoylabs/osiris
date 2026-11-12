//! Free text, and the confidentiality scan every piece of it passes.
//!
//! The ground rule: nothing secret, and nothing that identifies a person
//! or a machine, goes into harness state. An instruction to agents lowers
//! the rate of leaks; this type is the enforcement. [`Text`] has one
//! constructor, [`Text::new`], and it scans. Events hold `Text`, so no
//! free-text field reaches the log without passing here.
//!
//! The scan is a table of patterns plus a local denylist, and is best
//! effort by design: it catches the formats secrets take and the obvious
//! identifiers, not prose that identifies someone without matching a
//! pattern. Patterns are regular expressions so that they read as data
//! and can be lifted from published scanner rule sets. The second line of defence is the two exits (export and
//! each item's diff), and the third is the two-part event format, which
//! lets text be removed without breaking the log.
//!
//! The one path that does not scan is deserialization, which reads text
//! back out of the log. That text passed the scan when it went in.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::refusal::Refusal;

/// What the scan found.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    /// A credential in a known format: an access key, an API token, a
    /// bearer token, a JWT, or a long high-entropy string.
    Secret,
    /// A PEM private-key block.
    PrivateKey,
    /// `password=`, `token:` and the like, followed by a value.
    CredentialAssignment,
    /// An email address.
    Email,
    /// An absolute path under a home directory or a drive root, which
    /// carries a username or identifies a machine.
    AbsolutePath,
    /// An entry from the local denylist: the person's name, a client's
    /// name, the home directory, whatever the configuration lists.
    Denylisted,
}

impl Category {
    /// The thing found, as a noun phrase for a refusal.
    pub fn noun(self) -> &'static str {
        match self {
            Self::Secret => "credential",
            Self::PrivateKey => "private key",
            Self::CredentialAssignment => "credential assignment",
            Self::Email => "email address",
            Self::AbsolutePath => "absolute path",
            Self::Denylisted => "denylisted term",
        }
    }

    /// A description for a refusal, naming the category and nothing of
    /// the match.
    pub fn description(self) -> &'static str {
        match self {
            Self::Secret => "what looks like a credential or token",
            Self::PrivateKey => "a private key block",
            Self::CredentialAssignment => "a credential assigned a value",
            Self::Email => "an email address",
            Self::AbsolutePath => "an absolute path that identifies a user or machine",
            Self::Denylisted => "a term on the local denylist",
        }
    }
}

/// The local part of the scan: terms that must never appear. It is
/// itself sensitive, so it lives in local configuration and is never
/// exported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    denylist: Vec<String>,
}

impl Policy {
    /// A policy with the given denylist. Matching is case-insensitive;
    /// empty entries are dropped.
    pub fn new<I, S>(denylist: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            denylist: denylist
                .into_iter()
                .map(|s| s.as_ref().trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// The number of denylist entries, for reporting; never the entries.
    pub fn denylist_len(&self) -> usize {
        self.denylist.len()
    }
}

/// Free text that has passed the confidentiality scan.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Text(String);

impl Text {
    /// Scan `raw` and wrap it. `field` names the field for the refusal,
    /// which never echoes the match.
    pub fn new(field: &str, raw: impl Into<String>, policy: &Policy) -> Result<Self, Refusal> {
        let raw = raw.into();
        match scan(&raw, policy) {
            None => Ok(Self(raw)),
            Some(category) => Err(Refusal::Confidential {
                field: field.to_owned(),
                category,
            }),
        }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Text({:?})", self.0)
    }
}

/// Scan a string. `None` means nothing was found. Exposed for the exits:
/// the export of state, and each item's diff and commit message.
pub fn scan(raw: &str, policy: &Policy) -> Option<Category> {
    if let Some((category, _)) = PATTERNS.iter().find(|(_, re)| re.is_match(raw)) {
        return Some(*category);
    }
    // The narrower hand patterns name the category best, so they go
    // first; the vendored rule set catches the formats they do not.
    if let Some((category, _)) = LATE_PATTERNS.iter().find(|(_, re)| re.is_match(raw)) {
        return Some(*category);
    }
    if crate::rules::find(raw).is_some() {
        return Some(Category::Secret);
    }
    if has_high_entropy_run(raw) {
        return Some(Category::Secret);
    }
    if has_denylisted(raw, policy) {
        return Some(Category::Denylisted);
    }
    None
}

fn compile(table: &[(Category, &str)]) -> Vec<(Category, Regex)> {
    table
        .iter()
        .map(|(category, pattern)| {
            let re = Regex::new(pattern).unwrap_or_else(|e| {
                panic!("confidentiality pattern {pattern:?} does not compile: {e}")
            });
            (*category, re)
        })
        .collect()
}

/// Formats a credential takes. Checked first, so a token inside an
/// assignment or a URL is reported as what it is. These are the shapes
/// secret scanners publish rules for; a format not listed here is what
/// the high-entropy check and the exits are for.
static PATTERNS: LazyLock<Vec<(Category, Regex)>> = LazyLock::new(|| {
    use Category::*;
    compile(&[
        (PrivateKey, r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        // AWS access key id.
        (Secret, r"\bAKIA[0-9A-Z]{16}\b"),
        // GitHub tokens, classic and fine-grained.
        (Secret, r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}"),
        (Secret, r"\bgithub_pat_[A-Za-z0-9_]{20,}"),
        // Anthropic, then OpenAI-style keys; the long run rules out words.
        (Secret, r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
        (Secret, r"\bsk-[A-Za-z0-9_-]{32,}"),
        // Slack.
        (Secret, r"\bxox[abpr]-[A-Za-z0-9-]{10,}"),
        // Google API key.
        (Secret, r"\bAIza[0-9A-Za-z_-]{35}"),
        // GitLab personal token.
        (Secret, r"\bglpat-[A-Za-z0-9_-]{20,}"),
        // A bearer token in a header.
        (Secret, r"\bBearer [A-Za-z0-9._~+/=-]{20,}"),
        // A JWT: three base64url segments, the first two JSON objects.
        (
            Secret,
            r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        ),
    ])
});

/// Identifiers, checked after the credential formats.
static LATE_PATTERNS: LazyLock<Vec<(Category, Regex)>> = LazyLock::new(|| {
    use Category::*;
    compile(&[
        // `password = …`, `token: "…"` and the like, with a value of at
        // least eight characters: short enough to catch real ones, long
        // enough that "token: see env" passes.
        (
            CredentialAssignment,
            r#"(?i)\b(?:password|passwd|secret|api[_-]?key|access_key|private_key|token)\b[ \t]*[=:][ \t]*["']?[^\s"']{8,}"#,
        ),
        (Email, r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        // A path under a home directory or a Windows profile carries a
        // username.
        // Written with `[/]` so the pattern does not contain the literal
        // it detects, and the exit scan passes this file.
        (
            AbsolutePath,
            r"(?:/Users[/]|/home[/]|/root[/]|[A-Za-z]:\\Users\\)",
        ),
    ])
});

/// A run of 32 or more token characters mixing upper case, lower case
/// and digits. Hex hashes are excluded: they are lower case throughout
/// and legitimately appear in text (commit and evidence ids).
fn has_high_entropy_run(s: &str) -> bool {
    static RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/=_-]{32,}").unwrap());
    RUN.find_iter(s).any(|m| {
        let run = m.as_str();
        run.chars().any(|c| c.is_ascii_uppercase())
            && run.chars().any(|c| c.is_ascii_lowercase())
            && run.chars().any(|c| c.is_ascii_digit())
            && !run.chars().all(|c| c.is_ascii_hexdigit())
    })
}

fn has_denylisted(s: &str, policy: &Policy) -> bool {
    if policy.denylist.is_empty() {
        return false;
    }
    let lower = s.to_lowercase();
    policy
        .denylist
        .iter()
        .any(|term| lower.contains(term.as_str()))
}

#[cfg(test)]
#[path = "tests/text.rs"]
mod tests;
