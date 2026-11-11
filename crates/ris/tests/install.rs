//! The person's ris is the last build that passed every check (D-48):
//! a merge only records the commit; the person's next command builds it
//! in a checkout outside the repository and installs it, so a seat's
//! work in progress is never installed and the install never goes stale.
//! A failed build leaves the last good program in place.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

fn ok(dir: &Path, agent: &str, args: &[&str]) -> Output {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn commit(dir: &Path, file: &str, item: &str) -> String {
    fs::write(dir.join(file), format!("{file}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {file}"),
            "-m",
            &format!("Item: {item}\nAgent: heron"),
        ],
    );
    git(dir, &["rev-parse", "HEAD"])
}

/// A project at width one whose "build" writes the commit it was built
/// from, and fails when a file named `broken` is in the tree.
fn setup() -> (PathBuf, PathBuf) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("ris-install-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let dir = base.join("proj");
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, "person", &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    // Paths outside the repository, assembled at runtime: absolute
    // paths are never written into the tree.
    let to = base.join("bin").join("ris");
    let build = base.join("build");
    fs::write(
        dir.join(".osiris/config.toml"),
        format!(
            "denylist = [\"example client\"]\ndocs_item = false\n\
             [install]\n\
             build = \"test ! -e broken && mkdir -p out && git rev-parse HEAD > out/ris\"\n\
             artifact = \"out/ris\"\n\
             to = {to:?}\n\
             dir = {build:?}\n\
             [agents.heron]\nrole = \"lead\"\n",
            to = to.to_string_lossy(),
            build = build.to_string_lossy(),
        ),
    )
    .unwrap();
    ok(
        &dir,
        "person",
        &["phase", "open", "install the last good build"],
    );
    git(&dir, &["switch", "-q", "phase/P-1"]);
    ok(&dir, "person", &["item", "add", "first", "--check", "ok"]);
    ok(&dir, "person", &["item", "add", "second", "--check", "ok"]);
    (dir, base)
}

fn installed(base: &Path) -> Option<String> {
    fs::read_to_string(base.join("bin").join("ris"))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_persons_ris_is_the_last_merged_build_and_never_work_in_progress() {
    let (dir, base) = setup();

    // Nothing merged: nothing to install, and nothing is.
    let out = ris(&dir, "person", &["install"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no item has been merged yet"),
        "{}",
        stderr(&out)
    );
    assert_eq!(installed(&base), None);

    // A seat merges: it records the commit and installs nothing.
    ok(&dir, "heron", &["claim", "I-1"]);
    ok(&dir, "heron", &["done", "I-1"]);
    let first = commit(&dir, "first.txt", "I-1");
    ok(&dir, "heron", &["merge", "I-1"]);
    assert_eq!(installed(&base), None, "a seat's ris never installs");
    let refused = ris(&dir, "heron", &["install"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("the person's"),
        "{}",
        stderr(&refused)
    );

    // The person's next command, whatever it is, installs that build.
    let out = ok(&dir, "person", &["inbox"]);
    assert!(
        stderr(&out).contains("installed ris from") && stderr(&out).contains("I-1"),
        "{}",
        stderr(&out)
    );
    assert_eq!(installed(&base).as_deref(), Some(first.as_str()));
    // Once current, it does nothing more.
    let out = ok(&dir, "person", &["inbox"]);
    assert!(!stderr(&out).contains("installed"), "{}", stderr(&out));

    // Work in progress, committed but not merged, is never installed;
    // stray files in the build's checkout do not survive a build.
    fs::write(base.join("build/src/stray"), "x").unwrap();
    ok(&dir, "heron", &["claim", "I-2"]);
    let second = commit(&dir, "broken", "I-2");
    ok(&dir, "person", &["report"]);
    assert_eq!(installed(&base).as_deref(), Some(first.as_str()));
    let out = ok(&dir, "person", &["install"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("I-1"));
    assert_eq!(installed(&base).as_deref(), Some(first.as_str()));
    assert!(!base.join("build/src/stray").exists());
    assert!(!base.join("build/src/broken").exists());

    // A merged commit whose build fails: the command still runs, says
    // so once, and the last good program stays.
    ok(&dir, "heron", &["done", "I-2"]);
    ok(&dir, "heron", &["merge", "I-2"]);
    let out = ok(&dir, "person", &["report"]);
    assert!(
        stderr(&out).contains("warning: could not install") && stderr(&out).contains("ris install"),
        "{}",
        stderr(&out)
    );
    assert_eq!(installed(&base).as_deref(), Some(first.as_str()));
    let out = ok(&dir, "person", &["report"]);
    assert!(
        !stderr(&out).contains("warning"),
        "a failed build is not retried on every command"
    );
    // `ris install` retries it, and fails the same way.
    let out = ris(&dir, "person", &["install"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains(&second[..12]), "{}", stderr(&out));
    assert_eq!(installed(&base).as_deref(), Some(first.as_str()));

    let _ = fs::remove_dir_all(&base);
}
