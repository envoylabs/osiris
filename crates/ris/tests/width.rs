//! Several builders, each in its own git worktree, sharing one store.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        .envs(common::GIT_IDENTITY)
        .env("RIS_AGENT", "person")
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
/// one worktree per agent on its own branch from the phase branch,
/// outside the main tree. The main checkout stays on the main line: the
/// phase branch is checked out nowhere (D-43).
fn setup(name: &str, checks: &str, agents: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let base = std::env::temp_dir().join(format!("ris-width-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main = base.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
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
        format!("denylist = [\"example client\"]\ndocs_item = false\n{seats}"),
    )
    .unwrap();
    ok(&main, None, &["phase", "open", "width test"]);
    let trees = agents
        .iter()
        .map(|a| {
            let wt = base.join(a);
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    a,
                    wt.to_str().unwrap(),
                    "phase/P-1",
                ],
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

    // The main checkout's own directory is read-only during the merge,
    // as a seat's sandbox leaves it: no file can be added or replaced
    // there, while .git and .osiris inside it stay writable.
    let perms = |mode| fs::set_permissions(&main, fs::Permissions::from_mode(mode)).unwrap();
    perms(0o555);
    let out = ris(h, Some("heron"), &["merge", "I-1"]);
    perms(0o755);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        rev(&main, "phase/P-1"),
        head(h),
        "the phase branch moved to heron's commit"
    );
    // It moved by a ref update: the main checkout is still on the main
    // line with its files untouched, so a builder that cannot write
    // the main checkout can merge.
    assert_eq!(branch(&main), "main");
    assert!(!main.join("a.flag").exists());
    let after_first = rev(&main, "phase/P-1");

    let out = ris(w, Some("wren"), &["merge", "I-2"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a failing combined check exits 3"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("check flags FAILED"));
    assert_eq!(
        rev(&main, "phase/P-1"),
        after_first,
        "the phase branch did not move"
    );
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
    let after_first = rev(&main, "phase/P-1");
    let wren_before = head(w);
    let out = ris(w, Some("wren"), &["merge", "I-2"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("conflicts with work merged"));
    assert_eq!(
        rev(&main, "phase/P-1"),
        after_first,
        "the phase branch did not move"
    );
    assert_eq!(head(w), wren_before, "the failed rebase was undone");

    // A change that touches nothing merged before it goes through, on
    // top of heron's.
    ok(k, Some("kite"), &["merge", "I-3"]);
    assert_eq!(show(&main, "phase/P-1:README"), "heron's line\n");
    assert_eq!(show(&main, "phase/P-1:NEW"), "kite's file\n");
}

fn porcelain_of(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// A seat's sandbox refuses writes to a rules file, so a seat whose
/// branch is behind a change to one cannot rebase. The merge is refused
/// before the rebase starts, naming the file and the person's `ris up`,
/// and the worktree is left exactly as it was: same commit, same branch,
/// nothing modified and no rebase in progress. A read-only file stands
/// in for the sandbox.
#[test]
fn a_merge_behind_a_rules_file_change_is_refused_before_the_worktree_is_touched() {
    let (main, trees) = setup(
        "rules",
        "[checks.ok]\ncommand = \"true\"\n",
        &["heron", "wren"],
    );
    let (h, w) = (&trees[0], &trees[1]);
    for goal in ["add the rules", "a file", "change the rules", "another"] {
        ok(&main, None, &["item", "add", goal, "--check", "ok"]);
    }
    let land = |dir: &Path, item: &str, agent: &str, file: &str, text: &str| {
        ok(dir, Some(agent), &["claim", item]);
        fs::write(dir.join(file), text).unwrap();
        ok(dir, Some(agent), &["done", item]);
        commit(dir, item, agent);
    };
    // Both seats come to hold the rules file, then heron changes it.
    land(h, "I-1", "heron", "AGENTS.md", "rules, first\n");
    ok(h, Some("heron"), &["merge", "I-1"]);
    land(w, "I-2", "wren", "ONE", "one\n");
    ok(w, Some("wren"), &["merge", "I-2"]);
    land(h, "I-3", "heron", "AGENTS.md", "rules, second\n");
    ok(h, Some("heron"), &["merge", "I-3"]);
    let phase = rev(&main, "phase/P-1");

    land(w, "I-4", "wren", "TWO", "two\n");
    let before = head(w);
    let rules = w.join("AGENTS.md");
    let perms = |mode| fs::set_permissions(&rules, fs::Permissions::from_mode(mode)).unwrap();
    perms(0o444);
    let out = ris(w, Some("wren"), &["merge", "I-4"]);
    perms(0o644);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("phase/P-1 changed AGENTS.md"), "{err}");
    assert!(err.contains("`ris up wren`"), "{err}");
    assert_eq!(head(w), before, "the branch did not move");
    assert_eq!(branch(w), "wren", "no rebase was left in progress");
    assert_eq!(porcelain_of(w), "", "the worktree is untouched");
    assert_eq!(
        fs::read_to_string(&rules).unwrap(),
        "rules, first\n",
        "the rules file was not written"
    );
    assert_eq!(rev(&main, "phase/P-1"), phase, "the phase branch stayed");

    // Where the file can be written, the same merge goes through.
    ok(w, Some("wren"), &["merge", "I-4"]);
    assert_eq!(show(&main, "phase/P-1:AGENTS.md"), "rules, second\n");
    assert_eq!(show(&main, "phase/P-1:TWO"), "two\n");
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
    assert_eq!(show(&main, "phase/P-1:one"), "1\n");
    assert_eq!(show(&main, "phase/P-1:two"), "2\n");
    let log = Command::new("git")
        .args(["log", "--format=%s", "-3", "phase/P-1"])
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

fn rev(dir: &Path, what: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", what])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn show(dir: &Path, what: &str) -> String {
    let out = Command::new("git")
        .args(["show", what])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

fn branch(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Closing a phase merges its branch into the main line with a real
/// merge commit, after the checks pass on the merged result.
#[test]
fn closing_a_phase_merges_its_branch_into_the_main_line() {
    let (main, trees) = setup("close", "[checks.ok]\ncommand = \"true\"\n", &["heron"]);
    let h = &trees[0];
    ok(&main, None, &["item", "add", "a file", "--check", "ok"]);
    ok(h, Some("heron"), &["claim", "I-1"]);
    fs::write(h.join("a"), "a\n").unwrap();
    ok(h, Some("heron"), &["done", "I-1"]);
    commit(h, "I-1", "heron");
    ok(h, Some("heron"), &["merge", "I-1"]);
    let phase_tip = rev(&main, "phase/P-1");

    let out = ok(&main, None, &["phase", "close"]);
    assert!(out.contains("closing merge: phase/P-1 into main"), "{out}");
    assert!(out.contains("phase/P-1 deleted"), "{out}");
    assert_eq!(branch(&main), "main");
    assert_eq!(
        rev(&main, "HEAD^2"),
        phase_tip,
        "a merge commit whose second parent is the phase tip"
    );
    assert!(main.join("a").exists());
}

/// When the main line moved during the phase and the merged result
/// fails a check, nothing changes: the main line stays where it was,
/// the main checkout stays on it, and the phase stays open.
#[test]
fn a_closing_merge_that_fails_the_checks_changes_nothing() {
    let (main, trees) = setup(
        "close-fail",
        "[checks.flags]\ncommand = \"n=$(ls *.flag 2>/dev/null | wc -l); [ $n -le 1 ]\"\n",
        &["heron"],
    );
    let h = &trees[0];
    ok(&main, None, &["item", "add", "a flag", "--check", "flags"]);
    ok(h, Some("heron"), &["claim", "I-1"]);
    fs::write(h.join("a.flag"), "a\n").unwrap();
    ok(h, Some("heron"), &["done", "I-1"]);
    commit(h, "I-1", "heron");
    ok(h, Some("heron"), &["merge", "I-1"]);

    // Someone commits to the main line during the phase.
    fs::write(main.join("b.flag"), "b\n").unwrap();
    git(&main, &["add", "b.flag"]);
    git(&main, &["commit", "-q", "-m", "elsewhere"]);
    let main_before = rev(&main, "main");

    let r = ris(&main, None, &["phase", "close"]);
    assert_eq!(r.status.code(), Some(1));
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("fails flags") && err.contains("undone"),
        "{err}"
    );
    assert_eq!(
        rev(&main, "main"),
        main_before,
        "the main line did not move"
    );
    assert_eq!(branch(&main), "main", "the main checkout is still on main");
    let report = ok(&main, None, &["report"]);
    assert!(report.starts_with("P-1 open"), "{report}");
}

/// Phase branches are on by default: open branches from the main line,
/// close merges back and deletes the branch. Off, both leave branches
/// alone.
#[test]
fn phase_branches_follow_the_phase_and_can_be_turned_off() {
    let (main, _) = setup("branches", "[checks.ok]\ncommand = \"true\"\n", &[]);
    assert!(
        !rev(&main, "phase/P-1").is_empty(),
        "setup's phase open created the branch"
    );
    assert_eq!(branch(&main), "main", "and left the main checkout alone");
    ok(&main, None, &["phase", "close"]);
    assert_eq!(branch(&main), "main");
    let out = ok(&main, None, &["phase", "open", "second"]);
    assert!(out.contains("on branch phase/P-2"), "{out}");
    assert_eq!(branch(&main), "main");
    ok(&main, None, &["phase", "close"]);

    let config = main.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("phase_branches = false\n{c}")).unwrap();
    let out = ok(&main, None, &["phase", "open", "third"]);
    assert!(!out.contains("branch"), "{out}");
    assert_eq!(branch(&main), "main");
    let out = ok(&main, None, &["phase", "close"]);
    assert!(out.contains("already on main"), "{out}");
}

fn porcelain(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// The falsifier for "no residue": after `ris init --local` and a
/// builder's worktree, git sees no change at all in either tree.
#[test]
fn a_local_setup_leaves_nothing_for_git_to_see() {
    let base = std::env::temp_dir().join(format!("ris-width-local-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let repo = base.join("client");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("README"), "theirs\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "base"]);

    ok(&repo, None, &["init", "--local"]);
    assert_eq!(
        porcelain(&repo),
        "",
        "init --local changes nothing git can see"
    );
    // The denylist gate: no phase until the denylist is filled.
    let r = ris(&repo, None, &["phase", "open", "too soon"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stderr).contains("denylist"));
    assert!(!repo.join("OSIRIS.md").exists());
    assert!(!repo.join(".gitignore").exists());
    assert!(repo.join(".osiris/OSIRIS.md").exists());
    assert_eq!(
        fs::read_to_string(repo.join("CLAUDE.local.md")).unwrap(),
        "@.osiris/OSIRIS.md\n"
    );

    let out = ok(&repo, None, &["worktree", "heron"]);
    let wt = base.join("client-heron");
    assert!(out.contains("client-heron"), "{out}");
    assert_eq!(porcelain(&wt), "", "the worktree is clean too");
    let import = fs::read_to_string(wt.join("CLAUDE.local.md")).unwrap();
    assert!(
        import.starts_with('@') && import.trim_end().ends_with(".osiris/OSIRIS.md"),
        "{import}"
    );
    assert_eq!(porcelain(&repo), "");
    // The worktree finds the shared store.
    ok(&wt, Some("heron"), &["ready"]);
}

/// The remote-merge settings: without trailers, a plain commit merges;
/// without merge_on_close, closing leaves the main line and the phase
/// branch untouched for a merge on the git server.
#[test]
fn trailers_and_the_closing_merge_can_be_left_to_the_git_server() {
    let (main, trees) = setup("remote", "[checks.ok]\ncommand = \"true\"\n", &["heron"]);
    let config = main.join(".osiris/config.toml");
    let c = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!("trailers = false\nmerge_on_close = false\n{c}"),
    )
    .unwrap();
    let h = &trees[0];
    ok(&main, None, &["item", "add", "a file", "--check", "ok"]);
    ok(h, Some("heron"), &["claim", "I-1"]);
    fs::write(h.join("a"), "a\n").unwrap();
    let out = ok(h, Some("heron"), &["done", "I-1"]);
    assert!(!out.contains("trailer"), "{out}");
    git(h, &["add", "-A"]);
    git(
        h,
        &["commit", "-q", "-m", "Add a, in the project's own words"],
    );
    ok(h, Some("heron"), &["merge", "I-1"]);
    let main_before = rev(&main, "main");
    let out = ok(&main, None, &["phase", "close"]);
    assert!(out.contains("left to the git server"), "{out}");
    assert_eq!(rev(&main, "main"), main_before, "main did not move");
    assert_eq!(branch(&main), "main");
    assert!(
        !rev(&main, "phase/P-1").is_empty(),
        "the phase branch is kept"
    );
    assert!(ok(&main, None, &["report"]).starts_with("P-1 closed"));
}
