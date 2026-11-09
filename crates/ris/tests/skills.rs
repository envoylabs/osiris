//! Every `ris <command>` a shipped skill names is a real command, so the
//! skills cannot drift from the CLI unnoticed.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn every_ris_command_in_a_skill_exists() {
    let skills = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let mut checked = 0;
    for entry in fs::read_dir(&skills).unwrap() {
        let skill = entry.unwrap().path().join("SKILL.md");
        let Ok(text) = fs::read_to_string(&skill) else {
            continue;
        };
        for (i, _) in text.match_indices("`ris ") {
            let rest = &text[i + 5..];
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            if word.is_empty() || word.starts_with('-') {
                continue;
            }
            let out = Command::new(env!("CARGO_BIN_EXE_ris"))
                .args([word.as_str(), "--help"])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{} names `ris {word}`, which is not a command",
                skill.display()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 5,
        "only {checked} commands found; is the skill there?"
    );
}
