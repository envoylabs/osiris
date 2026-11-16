//! This repository's rules file is rendered, never written by hand: the
//! committed `AGENTS.md` is what `ris rules --render` gives from the
//! rules template and `docs/project-rules.md`, so a change to either
//! that is merged without rendering again fails here. Only the person
//! renders it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The project's own part and the rules file, from the repository root.
const SOURCE: &str = "docs/project-rules.md";
const RULES_FILE: &str = "AGENTS.md";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn the_committed_rules_file_is_what_the_command_renders() {
    let project = fs::read_to_string(repo().join(SOURCE)).unwrap();
    let committed = fs::read_to_string(repo().join(RULES_FILE)).unwrap();
    let rendered = ris::rules_file::render(SOURCE, RULES_FILE, &project);
    if committed != rendered {
        let line = committed
            .lines()
            .zip(rendered.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| committed.lines().count().min(rendered.lines().count()));
        panic!(
            "{RULES_FILE} is not what the rules and {SOURCE} render; they first differ at line {}:\n  \
             committed: {:?}\n  rendered:  {:?}\n  \
             next: the person runs `ris rules --render {SOURCE} --to {RULES_FILE}` at the repository root and commits the result",
            line + 1,
            committed.lines().nth(line).unwrap_or("(the file ends)"),
            rendered.lines().nth(line).unwrap_or("(the file ends)"),
        );
    }
}

/// The command writes that same file, for the person and nobody else.
#[test]
fn the_person_renders_a_rules_file_and_a_seat_is_refused() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ris-rules-render-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("docs")).unwrap();
    assert!(ris(&dir, "person", &["init"]).status.success());
    fs::write(dir.join(SOURCE), "# Ours\n\nOne rule of our own.\n").unwrap();
    let render = ["rules", "--render", SOURCE, "--to", RULES_FILE];

    // A seat never writes its own rules, whatever its role.
    for seat in ["heron", "wren"] {
        let out = ris(&dir, seat, &render);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{seat} rendered the rules");
        assert!(err.contains("only the person renders"), "{err}");
        assert!(!dir.join(RULES_FILE).exists());
    }

    let out = ris(&dir, "person", &render);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("rendered AGENTS.md from the rules and docs/project-rules.md"),
        "{text}"
    );
    let written = fs::read_to_string(dir.join(RULES_FILE)).unwrap();
    assert_eq!(
        written,
        ris::rules_file::render(SOURCE, RULES_FILE, "# Ours\n\nOne rule of our own.\n")
    );
    // The rules ris prints are in it whole, and the project's part.
    let rules = String::from_utf8(ris(&dir, "person", &["rules"]).stdout).unwrap();
    assert!(written.contains(rules.trim_end()), "the template, whole");
    assert!(written.ends_with("One rule of our own.\n"), "{written}");

    // Again changes nothing, and says so.
    let out = ris(
        &dir,
        "person",
        &["--json", "rules", "--render", SOURCE, "--to", RULES_FILE],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["changed"], false, "{v}");
    assert_eq!(v["data"]["rendered"], RULES_FILE);

    // A path outside the directory is refused, and nothing is written
    // there. Built from pieces: no rooted path stands in the tree.
    let outside = format!("..{}escaped.md", '/');
    let out = ris(
        &dir,
        "person",
        &["rules", "--render", SOURCE, "--to", &outside],
    );
    assert!(!out.status.success());
    assert!(!dir.with_file_name("escaped.md").exists());
    let _ = fs::remove_dir_all(&dir);
}
