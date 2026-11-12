//! What every test that runs `ris` shares.

/// A fixed git identity for the `ris` a test runs. `ris` itself makes
/// commits (the closing merge, the rebase inside `ris merge`), and a
/// machine with no identity configured, such as a CI runner, would
/// otherwise refuse them. The environment is used rather than a config
/// file so the test's repository stays as a real one would be.
pub const GIT_IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "ris"),
    ("GIT_AUTHOR_EMAIL", "ris@localhost"),
    ("GIT_COMMITTER_NAME", "ris"),
    ("GIT_COMMITTER_EMAIL", "ris@localhost"),
];
