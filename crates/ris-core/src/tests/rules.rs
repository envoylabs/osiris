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
