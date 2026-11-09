//! Free text, and the confidentiality scan every piece of it passes.
//!
//! The ground rule: nothing secret, and nothing that identifies a person
//! or a machine, goes into harness state. An instruction to agents lowers
//! the rate of leaks; this type is the enforcement. [`Text`] has one
//! constructor, [`Text::new`], and it scans. Events hold `Text`, so no
//! free-text field reaches the log without passing here.
//!
//! The scan is patterns plus a local denylist, and is best effort by
//! design: it catches the formats secrets take and the obvious
//! identifiers, not prose that identifies someone without matching a
//! pattern. The second line of defence is the two exits (export and
//! each item's diff), and the third is the two-part event format, which
//! lets text be removed without breaking the log.
//!
//! The one path that does not scan is deserialization, which reads text
//! back out of the log. That text passed the scan when it went in.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::refusal::Refusal;

/// What the scan found.
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
    if has_private_key_block(raw) {
        return Some(Category::PrivateKey);
    }
    if has_known_token(raw) || has_high_entropy_run(raw) {
        return Some(Category::Secret);
    }
    if has_credential_assignment(raw) {
        return Some(Category::CredentialAssignment);
    }
    if has_email(raw) {
        return Some(Category::Email);
    }
    if has_identifying_path(raw) {
        return Some(Category::AbsolutePath);
    }
    if has_denylisted(raw, policy) {
        return Some(Category::Denylisted);
    }
    None
}

fn has_private_key_block(s: &str) -> bool {
    s.contains("-----BEGIN") && s.contains("PRIVATE KEY")
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

/// The length of the token-character run starting at byte `at`.
fn run_len(s: &str, at: usize) -> usize {
    s[at..].chars().take_while(|&c| is_token_char(c)).count()
}

/// True when the byte before `at` is not a token character, so a prefix
/// found at `at` starts a token rather than sitting inside a word.
fn at_token_start(s: &str, at: usize) -> bool {
    s[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !is_token_char(c))
}

fn has_known_token(s: &str) -> bool {
    // Prefix, and the minimum run length after it, for formats whose
    // prefix alone identifies them.
    const PREFIXED: &[(&str, usize)] = &[
        ("AKIA", 16), // AWS access key id
        ("ghp_", 20), // GitHub personal token
        ("gho_", 20), // GitHub OAuth token
        ("ghu_", 20), // GitHub user-to-server token
        ("ghs_", 20), // GitHub server-to-server token
        ("ghr_", 20), // GitHub refresh token
        ("github_pat_", 20),
        ("sk-ant-", 20), // Anthropic API key
        ("sk-", 32),     // OpenAI-style key; the run is long enough to rule out words
        ("xoxb-", 10),   // Slack tokens
        ("xoxp-", 10),
        ("xoxa-", 10),
        ("xoxr-", 10),
        ("AIza", 35),   // Google API key
        ("glpat-", 20), // GitLab personal token
    ];
    for (prefix, min_after) in PREFIXED {
        let mut from = 0;
        while let Some(pos) = s[from..].find(prefix) {
            let at = from + pos;
            if at_token_start(s, at) && run_len(s, at) >= prefix.len() + min_after {
                return true;
            }
            from = at + prefix.len();
        }
    }
    // A bearer token: the word, a space, and a long run.
    let mut from = 0;
    while let Some(pos) = s[from..].find("Bearer ") {
        let at = from + pos + "Bearer ".len();
        if run_len(s, at) >= 20 {
            return true;
        }
        from = at;
    }
    // A JWT: three base64url segments, the first two starting `eyJ`.
    let mut from = 0;
    while let Some(pos) = s[from..].find("eyJ") {
        let at = from + pos;
        if at_token_start(s, at) {
            let run: String = s[at..].chars().take_while(|&c| is_token_char(c)).collect();
            let parts: Vec<&str> = run.split('.').collect();
            if parts.len() >= 3
                && parts[1].starts_with("eyJ")
                && parts.iter().take(3).all(|p| p.len() >= 8)
            {
                return true;
            }
        }
        from = at + 3;
    }
    false
}

/// A run of 32 or more token characters mixing upper case, lower case
/// and digits. Hex hashes are excluded: they are lower case throughout
/// and legitimately appear in text (commit and evidence ids).
fn has_high_entropy_run(s: &str) -> bool {
    let mut run = String::new();
    let check = |run: &str| -> bool {
        run.len() >= 32
            && run.chars().any(|c| c.is_ascii_uppercase())
            && run.chars().any(|c| c.is_ascii_lowercase())
            && run.chars().any(|c| c.is_ascii_digit())
            && !run.chars().all(|c| c.is_ascii_hexdigit())
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            run.push(c);
        } else {
            if check(&run) {
                return true;
            }
            run.clear();
        }
    }
    check(&run)
}

fn has_credential_assignment(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "api_key",
        "apikey",
        "api-key",
        "access_key",
        "private_key",
        "token",
    ];
    let lower = s.to_lowercase();
    for kw in KEYWORDS {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(kw) {
            let after = from + pos + kw.len();
            let rest = &lower[after..];
            let rest = rest.trim_start_matches([' ', '\t']);
            if let Some(rest) = rest.strip_prefix(['=', ':']) {
                let rest = rest.trim_start_matches([' ', '\t', '"', '\'']);
                let value_len = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\''))
                    .count();
                if value_len >= 8 {
                    return true;
                }
            }
            from = after;
        }
    }
    false
}

fn has_email(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'@' {
            continue;
        }
        let local_ok = s[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'));
        if !local_ok {
            continue;
        }
        let domain: String = s[i + 1..]
            .chars()
            .take_while(|&c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            .collect();
        let domain = domain.trim_end_matches('.');
        if let Some((_, tld)) = domain.rsplit_once('.')
            && tld.len() >= 2
            && tld.chars().all(|c| c.is_ascii_alphabetic())
        {
            return true;
        }
    }
    false
}

fn has_identifying_path(s: &str) -> bool {
    const PREFIXES: &[&str] = &["/Users/", "/home/", "/root/", "C:\\Users\\", "c:\\Users\\"];
    PREFIXES.iter().any(|p| s.contains(p))
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
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy::new(["Example Person", "acme-client"])
    }

    fn refused(raw: &str) -> Category {
        match Text::new("rationale", raw, &policy()) {
            Err(Refusal::Confidential { category, .. }) => category,
            Ok(_) => panic!("expected a refusal for {raw:?}"),
        }
    }

    fn accepted(raw: &str) {
        assert!(
            Text::new("rationale", raw, &policy()).is_ok(),
            "expected acceptance for {raw:?}"
        );
    }

    #[test]
    fn canary_private_key() {
        assert_eq!(
            refused("-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----"),
            Category::PrivateKey
        );
    }

    #[test]
    fn canary_known_tokens() {
        assert_eq!(refused("key AKIAIOSFODNN7EXAMPLE here"), Category::Secret);
        assert_eq!(
            refused("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123"),
            Category::Secret
        );
        assert_eq!(
            refused("sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"),
            Category::Secret
        );
        assert_eq!(refused("xoxb-123456789012-abcdefghij"), Category::Secret);
        assert_eq!(
            refused("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123"),
            Category::Secret
        );
        assert_eq!(
            refused("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV"),
            Category::Secret
        );
    }

    #[test]
    fn canary_high_entropy_run() {
        assert_eq!(
            refused("value Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MEFCQ0RFRkc="),
            Category::Secret
        );
    }

    #[test]
    fn canary_credential_assignment() {
        assert_eq!(
            refused("set password = hunter2hunter2"),
            Category::CredentialAssignment
        );
        assert_eq!(
            refused("TOKEN: \"abcdefghijkl\""),
            Category::CredentialAssignment
        );
    }

    #[test]
    fn canary_email() {
        assert_eq!(
            refused("mail someone.else@example.org about it"),
            Category::Email
        );
    }

    #[test]
    fn canary_absolute_paths() {
        assert_eq!(
            refused("see /Users/someone/src/x.rs"),
            Category::AbsolutePath
        );
        assert_eq!(
            refused("see /home/someone/src/x.rs"),
            Category::AbsolutePath
        );
        assert_eq!(
            refused("see C:\\Users\\someone\\x.rs"),
            Category::AbsolutePath
        );
    }

    #[test]
    fn canary_denylist_is_case_insensitive() {
        assert_eq!(
            refused("discussed with example person"),
            Category::Denylisted
        );
        assert_eq!(refused("the ACME-Client repo"), Category::Denylisted);
    }

    #[test]
    fn ordinary_prose_passes() {
        accepted("Use the claim in crates/ris-core/src/text.rs:12 and rerun the checks.");
        accepted("The task-force decided; see commit 3f2a9c1e7b6d5a4f3e2d1c0b9a8f7e6d5c4b3a29.");
        accepted("Evidence E-ab12cd34 attached; the token count is 1,200.");
        accepted("Mention @heron in the inbox; the token: see env.");
        accepted("Everything lives under ~/projects, relative paths only.");
        accepted("A hex id 0123456789abcdef0123456789abcdef0123456789abcdef is fine.");
    }

    #[test]
    fn refusals_do_not_echo_the_match() {
        let canaries = [
            "AKIAIOSFODNN7EXAMPLE",
            "someone.else@example.org",
            "/Users/someone/src",
            "password = hunter2hunter2",
            "example person",
        ];
        for canary in canaries {
            let refusal = Text::new("rationale", format!("x {canary} y"), &policy()).unwrap_err();
            let rendered = format!("{refusal} {}", refusal.next_step());
            for window in canary
                .as_bytes()
                .windows(6)
                .map(|w| std::str::from_utf8(w).unwrap())
            {
                assert!(
                    !rendered.contains(window),
                    "refusal {rendered:?} echoes {window:?}"
                );
            }
            assert!(rendered.contains("rationale"), "refusal names the field");
        }
    }

    #[test]
    fn text_serializes_as_a_plain_string() {
        let t = Text::new("goal", "ship it", &Policy::default()).unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"ship it\"");
        let back: Text = serde_json::from_str("\"ship it\"").unwrap();
        assert_eq!(back, t);
    }
}
