//! `ris phase close` where the closing merge is left to the git server
//! (`merge_on_close = false`): the phase stays open until its work is
//! on the local main line, however the server merged it, and the close
//! records where, so the closed phase's map is read from those commits
//! with its branch gone and whatever the server's merge is titled.
//!
//! A bare repository stands in for the server, and a second clone of it
//! for whatever merges there. `ris` reaches neither: the test pushes
//! and pulls, as the person does.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
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

fn data(dir: &Path, args: &[&str]) -> Value {
    let mut a = vec!["--json"];
    a.extend_from_slice(args);
    let v: Value = serde_json::from_str(&ok(dir, "person", &a)).unwrap();
    v["data"].clone()
}

/// Why `ris` refused, having failed.
fn refused(dir: &Path, args: &[&str]) -> String {
    let out = ris(dir, "person", args);
    assert!(!out.status.success(), "ris {} passed", args.join(" "));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The library every repository here starts with: twelve lines, so a
/// change to the first and one to the last are far enough apart to
/// merge with no conflict.
fn library(first: &str, last: &str) -> String {
    let mut text = format!("pub fn one() {{{first}}}\n");
    for n in 2..12 {
        text.push_str(&format!("pub fn f{n}() {{}}\n"));
    }
    text.push_str(&format!("pub fn twelve() {{{last}}}\n"));
    text
}

/// The three repositories of a test.
struct Repos {
    /// Everything of this test's, removed at its end.
    base: PathBuf,
    /// The person's repository, with the project in it.
    here: PathBuf,
    /// A clone of the server's, where the server's merges are made.
    there: PathBuf,
}

impl Drop for Repos {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// A server holding one commit on `main`, the person's clone of it with
/// a project at width one that leaves the closing merge to the server,
/// and the clone that merges there.
fn setup(name: &str) -> Repos {
    let base = std::env::temp_dir().join(format!("ris-server-merge-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", "server.git"]);
    let here = base.join("here");
    git(&base, &["clone", "-q", "server.git", "here"]);
    git(&here, &["switch", "-q", "-c", "main"]);
    fs::write(here.join("lib.rs"), library("", "")).unwrap();
    git(&here, &["add", "-A"]);
    git(&here, &["commit", "-q", "-m", "base"]);
    git(&here, &["push", "-q", "origin", "main"]);
    git(&base, &["clone", "-q", "server.git", "there"]);
    ok(&here, "person", &["init", "--local"]);
    fs::write(
        here.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::write(
        here.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\nmerge_on_close = false\n\
         [agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    Repos {
        there: base.join("there"),
        here,
        base,
    }
}

/// One item of the open phase, worked and merged: `file` written as
/// `text` in one commit on the phase's branch.
fn item(here: &Path, id: &str, file: &str, text: &str) {
    ok(here, "person", &["item", "add", file, "--check", "ok"]);
    ok(here, "heron", &["claim", id]);
    ok(here, "heron", &["done", id]);
    fs::write(here.join(file), text).unwrap();
    git(here, &["add", "-A"]);
    git(
        here,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: {file}"),
            "-m",
            &format!("Item: {id}\nAgent: heron"),
        ],
    );
    ok(here, "heron", &["merge", id]);
}

/// A phase opened on `phase/P-1` and worked: a file added by one item
/// and the library's last line changed by another, the branch pushed,
/// and the main checkout back on the main line. Returns the branch's
/// tip.
fn worked(r: &Repos) -> String {
    ok(&r.here, "person", &["phase", "open", "the work"]);
    git(&r.here, &["switch", "-q", "phase/P-1"]);
    item(&r.here, "I-1", "work.rs", "pub fn work() {}\n");
    item(&r.here, "I-2", "lib.rs", &library("", " /* done */ "));
    git(&r.here, &["switch", "-q", "main"]);
    git(&r.here, &["push", "-q", "origin", "phase/P-1"]);
    git(&r.here, &["rev-parse", "phase/P-1"])
}

/// Someone else's change reaches the server's main line while the
/// phase runs: the library's first line.
fn moved_on(r: &Repos) {
    fs::write(r.there.join("lib.rs"), library(" /* theirs */ ", "")).unwrap();
    git(&r.there, &["commit", "-q", "-am", "feat: theirs"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
}

/// The person pulls the main line.
fn pull(r: &Repos) {
    git(&r.here, &["pull", "-q", "--ff-only", "origin", "main"]);
}

fn node<'a>(doc: &'a Value, path: &str) -> &'a Value {
    doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == path)
        .unwrap_or_else(|| panic!("no node {path} in {doc}"))
}

/// The closed phase's map is its own work and nothing else: the file
/// it added and the one line of the library it changed, between the
/// commits given, with its branch gone.
fn mapped(r: &Repos, base: &str, tip: &str) {
    assert_eq!(
        git(&r.here, &["branch", "--format=%(refname:short)"]),
        "main",
        "the phase's branch is gone"
    );
    let map = data(&r.here, &["map", "P-1"]);
    assert_eq!(map["tip_ref"], "the work of P-1 on main", "{map}");
    assert_eq!(map["tip"], json!(tip), "{map}");
    assert_eq!(map["base"], json!(base), "{map}");
    assert_eq!(map["totals"]["changed"], 2, "{map}");
    assert_eq!(node(&map, "work.rs")["status"], "added");
    let lib = node(&map, "lib.rs");
    assert_eq!((&lib["added"], &lib["removed"]), (&json!(1), &json!(1)));
    assert_eq!(data(&r.here, &["report"])["phase"]["state"], "closed");
}

/// The close before the server has merged: refused, with the branch to
/// push and what to do, and the phase still open to take a fix.
fn refused_until_merged(r: &Repos) {
    let err = refused(&r.here, &["phase", "close", "--unreviewed"]);
    assert!(
        err.contains("phase/P-1's work is not on main in this repository, so P-1 stays open"),
        "{err}"
    );
    assert!(
        err.contains(
            "next: push phase/P-1, have it merged into main on the server, pull main, \
             then `ris phase close` again"
        ),
        "{err}"
    );
    assert!(
        err.contains("`ris phase close --merged-at <commit>`"),
        "{err}"
    );
    assert_eq!(data(&r.here, &["report"])["phase"]["state"], "open");
    assert!(
        git(&r.here, &["branch", "--list", "phase/P-1"]).contains("phase/P-1"),
        "the branch is as it was"
    );
}

#[test]
fn a_merge_commit_on_the_server_closes_the_phase_whatever_its_title() {
    let r = setup("merge");
    let tip = worked(&r);

    // The review names the branch to push; it pushes nothing.
    let review = ok(&r.here, "person", &["phase", "review"]);
    assert!(
        review.contains("next: push phase/P-1, have it merged into main on the git server"),
        "{review}"
    );
    refused_until_merged(&r);

    // The server's merge, titled as a server titles one.
    moved_on(&r);
    git(&r.there, &["pull", "-q", "origin", "main"]);
    git(&r.there, &["fetch", "-q", "origin"]);
    let before = git(&r.there, &["rev-parse", "main"]);
    git(
        &r.there,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge pull request #7 from phase/P-1",
            "origin/phase/P-1",
        ],
    );
    let merge = git(&r.there, &["rev-parse", "main"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
    pull(&r);

    // Reviewed at the branch's tip, which has not moved: it closes with
    // no flag.
    let closed = ok(&r.here, "person", &["phase", "close"]);
    assert!(
        closed.contains(&format!(
            "closing merge: made elsewhere; P-1's work is on main by the merge {}; phase/P-1 deleted",
            &merge[..12]
        )),
        "{closed}"
    );
    assert!(
        git(&r.here, &["merge-base", "--is-ancestor", &tip, "main"]).is_empty(),
        "the branch's commits are on the main line"
    );
    mapped(&r, &before, &merge);
}

#[test]
fn a_fast_forward_on_the_server_closes_the_phase_from_its_first_commit() {
    let r = setup("forward");
    let before = git(&r.here, &["rev-parse", "main"]);
    let tip = worked(&r);
    refused_until_merged(&r);

    git(&r.there, &["fetch", "-q", "origin"]);
    git(&r.there, &["merge", "-q", "--ff-only", "origin/phase/P-1"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
    pull(&r);

    let first = git(
        &r.here,
        &["rev-list", "--reverse", &format!("{before}..{tip}")],
    );
    let first = first.lines().next().unwrap();
    let closed = ok(&r.here, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains(&format!(
            "P-1's work is on main fast-forwarded, from {} to {}; phase/P-1 deleted",
            &first[..12],
            &tip[..12]
        )),
        "{closed}"
    );
    mapped(&r, &before, &tip);
}

#[test]
fn a_rebase_on_the_server_closes_the_phase_on_the_commits_it_made() {
    let r = setup("rebase");
    let tip = worked(&r);

    // The review rule is as it was: not reviewed, the close is refused
    // for that, merged or not.
    let err = refused(&r.here, &["phase", "close"]);
    assert!(err.contains("P-1 has not been reviewed"), "{err}");
    refused_until_merged(&r);

    // The server's main line has moved, so the branch's commits are
    // made again on top of it, as other commits.
    moved_on(&r);
    git(&r.there, &["fetch", "-q", "origin"]);
    let before = git(&r.there, &["rev-parse", "main"]);
    git(
        &r.there,
        &["switch", "-q", "-c", "landing", "origin/phase/P-1"],
    );
    git(&r.there, &["rebase", "-q", "main"]);
    git(&r.there, &["switch", "-q", "main"]);
    git(&r.there, &["merge", "-q", "--ff-only", "landing"]);
    let last = git(&r.there, &["rev-parse", "main"]);
    let first = git(&r.there, &["rev-parse", "main~1"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
    pull(&r);
    assert!(
        !Command::new("git")
            .args(["merge-base", "--is-ancestor", &tip, "main"])
            .current_dir(&r.here)
            .status()
            .unwrap()
            .success(),
        "none of the branch's own commits is on the main line"
    );

    let closed = ok(&r.here, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains(&format!(
            "P-1's work is on main rebased, from {} to {}; phase/P-1 deleted",
            &first[..12],
            &last[..12]
        )),
        "{closed}"
    );
    mapped(&r, &before, &last);
}

#[test]
fn a_squash_on_the_server_closes_the_phase_on_the_one_commit() {
    let r = setup("squash");
    worked(&r);
    refused_until_merged(&r);

    // The main line has moved in the file the phase changed, elsewhere
    // in it: the one commit still makes the phase's change.
    moved_on(&r);
    git(&r.there, &["fetch", "-q", "origin"]);
    let before = git(&r.there, &["rev-parse", "main"]);
    git(&r.there, &["merge", "-q", "--squash", "origin/phase/P-1"]);
    git(&r.there, &["commit", "-q", "-m", "The work (#7)"]);
    let squash = git(&r.there, &["rev-parse", "main"]);
    // And moves on again after it: the commit is found, not assumed to
    // be the last.
    fs::write(r.there.join("later.rs"), "pub fn later() {}\n").unwrap();
    git(&r.there, &["add", "-A"]);
    git(&r.there, &["commit", "-q", "-m", "feat: later"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
    pull(&r);

    let closed = ok(&r.here, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains(&format!(
            "P-1's work is on main squashed into {}; phase/P-1 deleted",
            &squash[..12]
        )),
        "{closed}"
    );
    mapped(&r, &before, &squash);
}

#[test]
fn a_merge_the_close_cannot_recognise_is_named_by_the_person() {
    let r = setup("named");
    let tip = worked(&r);

    // A squash the server had to resolve: its commit makes the phase's
    // change and more.
    git(&r.there, &["fetch", "-q", "origin"]);
    let before = git(&r.there, &["rev-parse", "main"]);
    git(&r.there, &["merge", "-q", "--squash", "origin/phase/P-1"]);
    fs::write(
        r.there.join("work.rs"),
        "pub fn work() { /* resolved */ }\n",
    )
    .unwrap();
    git(&r.there, &["commit", "-q", "-am", "The work (#7)"]);
    let squash = git(&r.there, &["rev-parse", "main"]);
    git(&r.there, &["push", "-q", "origin", "main"]);
    pull(&r);

    // Not recognised: the phase stays open.
    refused_until_merged(&r);

    // A commit that is not on the main line, and a name that is no
    // commit, are refused, and the phase stays open.
    let err = refused(
        &r.here,
        &["phase", "close", "--unreviewed", "--merged-at", &tip],
    );
    assert!(
        err.contains(&format!("{} is not on the main line main", &tip[..12])),
        "{err}"
    );
    let err = refused(
        &r.here,
        &["phase", "close", "--unreviewed", "--merged-at", "no-such"],
    );
    assert!(err.contains("`no-such` names no commit"), "{err}");
    assert_eq!(data(&r.here, &["report"])["phase"]["state"], "open");

    // Named, it closes there.
    let closed = ok(
        &r.here,
        "person",
        &["phase", "close", "--unreviewed", "--merged-at", &squash],
    );
    assert!(
        closed.contains(&format!(
            "P-1's work is on main at {}, as named; phase/P-1 deleted",
            &squash[..12]
        )),
        "{closed}"
    );
    assert_eq!(
        git(&r.here, &["branch", "--format=%(refname:short)"]),
        "main"
    );
    let map = data(&r.here, &["map", "P-1"]);
    assert_eq!(map["tip"], json!(squash), "{map}");
    assert_eq!(map["base"], json!(before), "{map}");
    assert_eq!(map["totals"]["changed"], 2, "{map}");
}

#[test]
fn a_commit_is_named_only_where_the_merge_is_made_elsewhere() {
    let r = setup("here");
    // The closing merge made here, as the default has it.
    let config = r.here.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("merge_on_close = false\n", ""),
    )
    .unwrap();
    worked(&r);
    let main = git(&r.here, &["rev-parse", "main"]);
    let err = refused(
        &r.here,
        &["phase", "close", "--unreviewed", "--merged-at", &main],
    );
    assert!(
        err.contains("this project's closing merge is made here"),
        "{err}"
    );
    assert_eq!(data(&r.here, &["report"])["phase"]["state"], "open");

    // And made here it is found as it was: by its title, with no
    // commits recorded.
    let closed = ok(&r.here, "person", &["phase", "close", "--unreviewed"]);
    assert!(
        closed.contains("closing merge: phase/P-1 into main at "),
        "{closed}"
    );
    let map = data(&r.here, &["map", "P-1"]);
    assert_eq!(map["tip_ref"], "the closing merge of P-1 on main", "{map}");
    assert_eq!(map["totals"]["changed"], 2, "{map}");
}
