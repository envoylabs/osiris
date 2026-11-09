//! Published secret-detection rules: gitleaks' default rule set,
//! vendored in `rules/` with its licence and the tag it came from.
//!
//! The file is embedded at build time and parsed at first use. Each
//! rule's regex is compiled only when a text contains one of the rule's
//! keywords, so a scan pays for the rules that could apply and no more.

use std::sync::{LazyLock, OnceLock};

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

/// The rule set, as shipped.
pub const SOURCE: &str = include_str!("../rules/gitleaks.toml");

/// Where it came from, for anyone auditing the scan.
pub const PROVENANCE: &str = "gitleaks v8.18.4, config/gitleaks.toml, MIT";

#[derive(Deserialize)]
struct File {
    rules: Vec<RawRule>,
}

#[derive(Deserialize)]
struct RawRule {
    id: String,
    regex: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    entropy: Option<f64>,
    #[serde(rename = "secretGroup")]
    secret_group: Option<usize>,
    allowlist: Option<RawAllow>,
}

#[derive(Deserialize, Default)]
struct RawAllow {
    #[serde(default)]
    regexes: Vec<String>,
    #[serde(default)]
    stopwords: Vec<String>,
}

struct Rule {
    id: String,
    pattern: String,
    keywords: Vec<String>,
    entropy: Option<f64>,
    secret_group: Option<usize>,
    allow_regexes: Vec<String>,
    stopwords: Vec<String>,
    compiled: OnceLock<Option<Compiled>>,
}

struct Compiled {
    regex: Regex,
    allow: Vec<Regex>,
}

fn build(pattern: &str) -> Option<Regex> {
    RegexBuilder::new(pattern).size_limit(64 << 20).build().ok()
}

impl Rule {
    fn compiled(&self) -> Option<&Compiled> {
        self.compiled
            .get_or_init(|| {
                Some(Compiled {
                    regex: build(&self.pattern)?,
                    allow: self.allow_regexes.iter().filter_map(|r| build(r)).collect(),
                })
            })
            .as_ref()
    }
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let file: File = toml::from_str(SOURCE).expect("the vendored rule set parses");
    file.rules
        .into_iter()
        .filter_map(|r| {
            let allow = r.allowlist.unwrap_or_default();
            Some(Rule {
                id: r.id,
                pattern: r.regex?,
                keywords: r.keywords.into_iter().map(|k| k.to_lowercase()).collect(),
                entropy: r.entropy,
                secret_group: r.secret_group,
                allow_regexes: allow.regexes,
                stopwords: allow
                    .stopwords
                    .into_iter()
                    .map(|w| w.to_lowercase())
                    .collect(),
                compiled: OnceLock::new(),
            })
        })
        .collect()
});

/// Shannon entropy of `s`, in bits per character.
fn entropy(s: &str) -> f64 {
    let n = s.chars().count() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    counts
        .values()
        .map(|&k| {
            let p = k as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// The id of the first rule that finds a secret in `text`, if any.
pub fn find(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    for rule in RULES.iter() {
        if !rule.keywords.is_empty() && !rule.keywords.iter().any(|k| lower.contains(k.as_str())) {
            continue;
        }
        let Some(c) = rule.compiled() else { continue };
        for caps in c.regex.captures_iter(text) {
            let group = rule
                .secret_group
                .unwrap_or(if caps.len() > 1 { 1 } else { 0 });
            // `found`, not `secret`: a line reading `secret = ...` is what
            // the credential-assignment pattern looks for.
            let Some(found) = caps.get(group).or_else(|| caps.get(0)) else {
                continue;
            };
            let found = found.as_str();
            if let Some(min) = rule.entropy
                && entropy(found) < min
            {
                continue;
            }
            let lowered = found.to_lowercase();
            if rule.stopwords.iter().any(|w| lowered.contains(w.as_str())) {
                continue;
            }
            if c.allow.iter().any(|a| a.is_match(found)) {
                continue;
            }
            return Some(rule.id.as_str());
        }
    }
    None
}

/// Rules whose regex the Rust regex crate cannot compile, by id.
pub fn uncompilable() -> Vec<&'static str> {
    RULES
        .iter()
        .filter(|r| r.compiled().is_none())
        .map(|r| r.id.as_str())
        .collect()
}

/// How many rules are loaded.
pub fn count() -> usize {
    RULES.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_set_loads_and_almost_all_of_it_compiles() {
        assert!(count() >= 170, "{} rules", count());
        let skipped = uncompilable();
        // Every skipped rule is named here, so a newer rule set that
        // loses rules to syntax fails this test rather than shrinking
        // the scan silently.
        let expected: &[&str] = &[];
        assert_eq!(skipped, expected, "rules Rust cannot compile");
    }

    // Canaries are assembled from pieces at runtime: no secret-shaped
    // literal may sit in the tree, since the exit scan has no
    // exceptions. These formats are covered by the vendored rules and
    // not by the patterns in text.rs.
    #[test]
    fn vendored_rules_find_formats_the_hand_patterns_do_not() {
        let stripe = ["sk_li", "ve_", "4eC39HqLyjWDarjtT1zdp7dc"].concat();
        assert_eq!(find(&format!("key {stripe}")), Some("stripe-access-token"));
        let shopify = ["shpat", "_", "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"].concat();
        assert_eq!(find(&shopify), Some("shopify-access-token"));
    }

    #[test]
    fn ordinary_text_finds_nothing() {
        for t in [
            "Use the claim in crates/ris-core/src/text.rs:12 and rerun the checks.",
            "the token count is 1,200 and the api is stable",
            "password policy: see the plan's Confidentiality section",
            "commit 3f2a9c1e7b6d5a4f3e2d1c0b9a8f7e6d5c4b3a29 merged",
        ] {
            assert_eq!(find(t), None, "{t}");
        }
    }

    #[test]
    fn entropy_is_shannon_bits_per_char() {
        assert_eq!(entropy("aaaa"), 0.0);
        assert!((entropy("ab") - 1.0).abs() < 1e-9);
    }
}
