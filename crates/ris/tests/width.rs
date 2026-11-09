//! Several builders, each in its own git worktree, sharing one store.

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
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {} failed: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A repository with a phase open, `checks` as its checks file, and
/// one worktree per agent on its own branch, outside the main tree.
fn setup(name: &str, checks: &str, agents: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let base = std::env::temp_dir().join(format!("ris-width-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main = base.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "phase"]);
    fs::write(main.join("README"), "base\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    ok(&main, None, &["init"]);
    fs::write(main.join(".osiris/checks.toml"), checks).unwrap();
    let seats: String = agents
        .iter()
        .map(|a| format!("[agents.{a}]\nrole = \"builder\"\n"))
        .collect();
    fs::write(
        main.join(".osiris/config.toml"),
        format!("denylist = []\n{seats}"),
    )
    .unwrap();
    ok(&main, None, &["phase", "open", "width test"]);
    let trees = agents
        .iter()
        .map(|a| {
            let wt = base.join(a);
            git(
                &main,
                &["worktree", "add", "-q", "-b", a, wt.to_str().unwrap()],
            );
            wt
        })
        .collect();
    (main, trees)
}

#[test]
fn a_worktree_finds_the_shared_store_and_checks_run_in_it() {
    let (main, trees) = setup(
        "store",
        "[checks.marker]\ncommand = \"test -f only-here.txt\"\n",
        &["heron"],
    );
    let wt = &trees[0];
    ok(
        &main,
        None,
        &["item", "add", "a thing", "--check", "marker"],
    );
    // The worktree is outside the main tree, yet sees the same store.
    let ready = ok(wt, Some("heron"), &["ready"]);
    assert!(ready.contains("I-1"), "{ready}");
    ok(wt, Some("heron"), &["claim", "I-1"]);
    // The marker exists only in the worktree, so the check passes only
    // if it runs there.
    fs::write(wt.join("only-here.txt"), "x\n").unwrap();
    assert!(!main.join("only-here.txt").exists());
    ok(wt, Some("heron"), &["done", "I-1"]);
    let report = ok(&main, None, &["report"]);
    assert!(report.contains("I-1   done"), "{report}");
}

/// Commit everything in `dir` with the item and agent trailers.
fn commit(dir: &Path, item: &str, agent: &str) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {item}"),
            "-m",
            &format!("Item: {item}\nAgent: {agent}"),
        ],
    );
}

fn head(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// The falsifier for the plan's "later merges cannot undo earlier
/// ones": two items that each pass alone but fail together. The second
/// merge runs the checks on the combined tree and is refused, and the
/// phase branch does not move.
#[test]
fn a_merge_that_breaks_the_combined_tree_is_refused() {
    let (main, trees) = setup(
        "combined",
        // At most one flag file may exist: each item adds one.
        "[checks.flags]\ncommand = \"n=$(ls *.flag 2>/dev/null | wc -l); [ $n -le 1 ]\"\n",
        &["heron", "wren"],
    );
    let (h, w) = (&trees[0], &trees[1]);
    ok(&main, None, &["item", "add", "a", "--check", "flags"]);
    ok(&main, None, &["item", "add", "b", "--check", "flags"]);
    ok(h, Some("heron"), &["claim", "I-1"]);
    ok(w, Some("wren"), &["claim", "I-2"]);

    fs::write(h.join("a.flag"), "a\n").unwrap();
    ok(h, Some("heron"), &["done", "I-1"]);
    commit(h, "I-1", "heron");
    fs::write(w.join("b.flag"), "b\n").unwrap();
    ok(w, Some("wren"), &["done", "I-2"]);
    commit(w, "I-2", "wren");

    ok(h, Some("heron"), &["merge", "I-1"]);
    assert_eq!(
        head(&main),
        head(h),
        "the phase branch moved to heron's commit"
    );
    let after_first = head(&main);

    let out = ris(w, Some("wren"), &["merge", "I-2"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a failing combined check exits 3"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("check flags FAILED"));
    assert_eq!(head(&main), after_first, "the phase branch did not move");
    let report = ok(&main, None, &["report"]);
    assert!(
        report.contains("I-1   merged") && report.contains("I-2   done"),
        "{report}"
    );
}

/// A textual conflict with work merged before it is refused, the
/// builder's rebase is undone, and nothing moves.
#[test]
fn a_conflicting_merge_is_refused_and_a_clean_one_goes_through() {
    let (main, trees) = setup(
        "conflict",
        "[checks.ok]\ncommand = \"true\"\n",
        &["heron", "wren", "kite"],
    );
    let (h, w, k) = (&trees[0], &trees[1], &trees[2]);
    for goal in ["edit readme", "edit readme too", "add a file"] {
        ok(&main, None, &["item", "add", goal, "--check", "ok"]);
    }
    ok(h, Some("heron"), &["claim", "I-1"]);
    ok(w, Some("wren"), &["claim", "I-2"]);
    ok(k, Some("kite"), &["claim", "I-3"]);

    fs::write(h.join("README"), "heron's line\n").unwrap();
    fs::write(w.join("README"), "wren's line\n").unwrap();
    fs::write(k.join("NEW"), "kite's file\n").unwrap();
    for (dir, item, agent) in [(h, "I-1", "heron"), (w, "I-2", "wren"), (k, "I-3", "kite")] {
        ok(dir, Some(agent), &["done", item]);
        commit(dir, item, agent);
    }

    ok(h, Some("heron"), &["merge", "I-1"]);
    let after_first = head(&main);
    let wren_before = head(w);
    let out = ris(w, Some("wren"), &["merge", "I-2"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("conflicts with work merged"));
    assert_eq!(head(&main), after_first, "the phase branch did not move");
    assert_eq!(head(w), wren_before, "the failed rebase was undone");

    // A change that touches nothing merged before it goes through, on
    // top of heron's.
    ok(k, Some("kite"), &["merge", "I-3"]);
    assert_eq!(
        fs::read_to_string(main.join("README")).unwrap(),
        "heron's line\n"
    );
    assert_eq!(
        fs::read_to_string(main.join("NEW")).unwrap(),
        "kite's file\n"
    );
}

/// Two merges submitted at the same moment land in one order: the
/// second is rebased onto and checked against a tree containing the
/// first, so the phase branch ends with both.
#[test]
fn simultaneous_merges_are_serialised() {
    let (main, trees) = setup(
        "simultaneous",
        // Slow enough that the two merges overlap if nothing stops them.
        "[checks.slow]\ncommand = \"sleep 1\"\n",
        &["heron", "wren"],
    );
    let (h, w) = (trees[0].clone(), trees[1].clone());
    ok(&main, None, &["item", "add", "one", "--check", "slow"]);
    ok(&main, None, &["item", "add", "two", "--check", "slow"]);
    ok(&h, Some("heron"), &["claim", "I-1"]);
    ok(&w, Some("wren"), &["claim", "I-2"]);
    fs::write(h.join("one"), "1\n").unwrap();
    fs::write(w.join("two"), "2\n").unwrap();
    ok(&h, Some("heron"), &["done", "I-1"]);
    commit(&h, "I-1", "heron");
    ok(&w, Some("wren"), &["done", "I-2"]);
    commit(&w, "I-2", "wren");

    let a = std::thread::spawn(move || ris(&h, Some("heron"), &["merge", "I-1"]));
    let b = std::thread::spawn(move || ris(&w, Some("wren"), &["merge", "I-2"]));
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    assert!(main.join("one").exists() && main.join("two").exists());
    let log = Command::new("git")
        .args(["log", "--format=%s", "-3"])
        .current_dir(&main)
        .output()
        .unwrap();
    let subjects = String::from_utf8(log.stdout).unwrap();
    assert_eq!(
        subjects.lines().count(),
        3,
        "base plus one commit per item: {subjects}"
    );
}
