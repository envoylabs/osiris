# Vendored secret-detection rules

`gitleaks.toml` is the default rule set of gitleaks, taken unmodified
from tag `v8.18.4` of the gitleaks repository (`config/gitleaks.toml`),
under the MIT licence in `gitleaks.LICENSE`.

`ris-core` loads it at first use (`src/rules.rs`) and applies each rule's
regex, keywords, entropy threshold, secret group, and rule allowlist
(regexes and stopwords). Path-based allowlists do not apply: the scan
sees text, not files. Rules whose regex the Rust `regex` crate cannot
compile are skipped, and a test names every one, so none drops out
silently.

To update: replace both files from a newer tag, change the tag here,
and run the tests.
