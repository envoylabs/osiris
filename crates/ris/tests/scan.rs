//! `ris scan` reads every file it is given, tracked or not, and never
//! reports clean having read nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
}

fn ris(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-scan-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    fs::write(dir.join(".gitignore"), "ignored.txt\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    assert!(ris(&dir, &["init", "--local"]).status.success());
    dir
}

// An absolute home path, assembled at runtime so the literal is never
// in the tree: the exit scan has no exceptions.
fn canary() -> String {
    format!("see {}", ["/Us", "ers/someone/notes.txt"].concat())
}

#[test]
fn untracked_and_explicitly_named_ignored_files_are_scanned() {
    let dir = repo("untracked");
    fs::write(dir.join("new.md"), canary()).unwrap();
    fs::write(dir.join("ignored.txt"), canary()).unwrap();

    let named = ris(&dir, &["scan", "new.md"]);
    assert_eq!(
        named.status.code(),
        Some(2),
        "a new, untracked file is read"
    );

    let whole = ris(&dir, &["scan"]);
    assert_eq!(
        whole.status.code(),
        Some(2),
        "the whole-tree scan reads untracked files too"
    );

    fs::remove_file(dir.join("new.md")).unwrap();
    assert!(
        ris(&dir, &["scan"]).status.success(),
        "an ignored file is not read unless named"
    );
    let ignored = ris(&dir, &["scan", "ignored.txt"]);
    assert_eq!(
        ignored.status.code(),
        Some(2),
        "a named file is read even if ignored"
    );
    let err = String::from_utf8_lossy(&ignored.stderr);
    assert!(!err.contains("someone"), "the match is not echoed");
}

#[test]
fn scanning_nothing_is_not_clean() {
    let dir = repo("nothing");
    fs::create_dir_all(dir.join("empty")).unwrap();
    let empty = ris(&dir, &["scan", "empty"]);
    assert!(!empty.status.success(), "an empty directory is not clean");
    assert!(!String::from_utf8_lossy(&empty.stdout).contains("clean"));

    let missing = ris(&dir, &["scan", "no-such-file.md"]);
    assert!(!missing.status.success(), "a typo is not clean");

    fs::write(dir.join("fine.md"), "nothing to see\n").unwrap();
    let fine = ris(&dir, &["scan", "fine.md"]);
    assert!(fine.status.success());
    assert!(String::from_utf8_lossy(&fine.stdout).contains("clean: 1 file(s)"));
}
