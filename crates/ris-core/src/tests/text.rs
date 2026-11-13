use super::*;

fn policy() -> Policy {
    Policy::new(["Example Person", "acme-client"])
}

/// Canaries are assembled from pieces at runtime so that no
/// secret-shaped literal sits in the tree: `ris merge` scans every
/// line a commit adds, with no allowance mechanism by design, and
/// would refuse the commit that added one. This is the contortion
/// the no-exceptions rule costs; it is confined to fixtures like
/// these.
fn canary(parts: &[&str]) -> String {
    parts.concat()
}

fn refused(raw: impl AsRef<str>) -> Category {
    let raw = raw.as_ref();
    match Text::new("rationale", raw, &policy()) {
        Err(Refusal::Confidential { category, .. }) => category,
        Err(other) => panic!("unexpected refusal {other:?} for {raw:?}"),
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
    let block = canary(&[
        "-----BEGIN RSA ",
        "PRIVATE KEY-----\nMIIE...\n-----END RSA ",
        "PRIVATE KEY-----",
    ]);
    assert_eq!(refused(block), Category::PrivateKey);
}

#[test]
fn canary_known_tokens() {
    assert_eq!(
        refused(canary(&["key AKIA", "IOSFODNN7EXAMPLE here"])),
        Category::Secret
    );
    assert_eq!(
        refused(canary(&[
            "ghp_",
            "ABCDEFGHIJKLMNOP",
            "QRSTUVWXYZabcdef0123"
        ])),
        Category::Secret
    );
    assert_eq!(
        refused(canary(&[
            "sk-ant-",
            "api03-ABCDEFGHIJKLMNOP",
            "QRSTUVWXYZ0123456789"
        ])),
        Category::Secret
    );
    assert_eq!(
        refused(canary(&["xoxb-", "123456789012-abcdefghij"])),
        Category::Secret
    );
    assert_eq!(
        refused(canary(&[
            "Authorization: Bearer ",
            "abcdefghijklmnopqrstuvwxyz0123"
        ])),
        Category::Secret
    );
    assert_eq!(
        refused(canary(&[
            "eyJhbGciOiJIUzI1NiJ9.",
            "eyJzdWIiOiIxMjM0In0.",
            "SflKxwRJSMeKKF2QT4",
            "fwpMeJf36POk6yJV"
        ])),
        Category::Secret
    );
}

#[test]
fn canary_high_entropy_run() {
    assert_eq!(
        refused(canary(&[
            "value Zm9vYmFyYmF6cXV4",
            "MTIzNDU2Nzg5MEFCQ0RFRkc="
        ])),
        Category::Secret
    );
}

#[test]
fn canary_credential_assignment() {
    assert_eq!(
        refused(canary(&["set pass", "word = hunter2hunter2"])),
        Category::CredentialAssignment
    );
    assert_eq!(
        refused(canary(&["TOK", "EN: \"abcdefghijkl\""])),
        Category::CredentialAssignment
    );
}

#[test]
fn canary_email() {
    assert_eq!(
        refused(canary(&["mail someone.else", "@example.org about it"])),
        Category::Email
    );
}

#[test]
fn canary_absolute_paths() {
    assert_eq!(
        refused(canary(&["see /Us", "ers/someone/src/x.rs"])),
        Category::AbsolutePath
    );
    assert_eq!(
        refused(canary(&["see /ho", "me/someone/src/x.rs"])),
        Category::AbsolutePath
    );
    assert_eq!(
        refused(canary(&["see C:\\Us", "ers\\someone\\x.rs"])),
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
        canary(&["AKIA", "IOSFODNN7EXAMPLE"]),
        canary(&["someone.else", "@example.org"]),
        canary(&["/Us", "ers/someone/src"]),
        canary(&["pass", "word = hunter2hunter2"]),
        "example person".to_owned(),
    ];
    for canary in canaries.iter().map(String::as_str) {
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
