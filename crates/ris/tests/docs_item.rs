//! The standing rule: every phase gets an item to bring the docs up to
//! date, and the phase does not close until it is merged or superseded.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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

fn ok(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_phase_closes_only_with_its_docs_item_settled() {
    let dir = std::env::temp_dir().join(format!("ris-docs-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, "person", &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();

    let out = ok(&dir, "person", &["phase", "open", "with docs"]);
    assert!(out.contains("I-1 brings the docs up to date"), "{out}");
    let item = ok(&dir, "person", &["show", "I-1"]);
    assert!(item.contains("Docs describe the system as built"), "{item}");
    assert!(
        item.contains("checks: test"),
        "every declared check: {item}"
    );

    let r = ris(&dir, "person", &["phase", "close"]);
    assert!(!r.status.success());
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("I-1 (the docs) is not merged"),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );

    // Superseded counts as settled: the person chose another item.
    ok(
        &dir,
        "person",
        &["item", "add", "docs, rewritten", "--no-checks"],
    );
    ok(&dir, "person", &["item", "supersede", "I-1", "--by", "I-2"]);
    ok(&dir, "person", &["phase", "close"]);

    // Turned off, a phase opens with no docs item.
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        c.replace("denylist = [", "docs_item = false\ndenylist = ["),
    )
    .unwrap();
    let out = ok(&dir, "person", &["phase", "open", "without docs"]);
    assert!(!out.contains("docs up to date"), "{out}");
    ok(&dir, "person", &["phase", "close"]);
}
