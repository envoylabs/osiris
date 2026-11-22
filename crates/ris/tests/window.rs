//! `ris window`: the command that holds the map page's window is the
//! person's alone, and a ris built without a window says so.
//!
//! No test here runs it as the person in a ris built with the `window`
//! feature: that would open a window. What the window serves is tested
//! in the window's own crate, and how `ris map` waits for it in the
//! map's unit tests.

use std::fs;
use std::path::{Path, PathBuf};
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

/// A project with a builder seat and a page to show.
fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-window-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    assert!(ris(&dir, "person", &["init"]).status.success());
    assert!(
        ris(
            &dir,
            "person",
            &["seat", "add", "rook", "--role", "builder"]
        )
        .status
        .success()
    );
    fs::write(dir.join("page.html"), "<!doctype html><p>a page</p>").unwrap();
    dir
}

#[test]
fn a_seat_is_refused_a_window_and_nothing_is_said_to_be_shown() {
    let dir = setup("seat");
    let out = ris(&dir, "rook", &["window", "page.html"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("a seat never opens one"), "{err}");
    assert!(err.contains("next: `ris map --list`"), "{err}");
    // `ris map` reads the first line of output for whether the page is
    // shown: a refusal leaves none.
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_command_is_not_listed_in_help() {
    let dir = setup("help");
    let out = ris(&dir, "person", &["--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("map"), "{help}");
    assert!(
        !help.lines().any(|l| l.trim_start().starts_with("window")),
        "{help}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(not(feature = "window"))]
#[test]
fn a_ris_built_without_a_window_says_so_and_how_to_build_one() {
    let dir = setup("without");
    let out = ris(&dir, "person", &["window", "page.html"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("built without a window"), "{err}");
    assert!(err.contains("--features window"), "{err}");
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    let _ = fs::remove_dir_all(&dir);
}
