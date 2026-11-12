//! `ris map`: the phase's change set as a graph document over the
//! source files of a small fixture repository, the page that embeds
//! it, and the same map of the phase once it is closed.

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

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A one-package Rust repository on `main` with a project in it: a
/// library of three modules, a document that is a contract, and files
/// that are neither source nor contract.
fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ris-map-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "Cargo.toml", "[package]\nname = \"fixture\"\n");
    write(&dir, "README.md", "a fixture\n");
    write(&dir, "docs/wire.md", "# The wire format\n\nOne frame.\n");
    write(
        &dir,
        "src/lib.rs",
        "pub mod old;\npub mod store;\npub mod wire;\n",
    );
    write(&dir, "src/old.rs", "pub fn gone() {}\n");
    write(
        &dir,
        "src/store.rs",
        "use std::fs;\n\nuse crate::wire::Frame;\n\npub fn save(_: Frame) {\n    let _ = fs::read(\"frames\");\n}\n",
    );
    write(&dir, "src/wire.rs", "pub struct Frame;\n// one\n");
    write(
        &dir,
        "tests/wire.rs",
        "use fixture::wire::Frame;\n\n#[test]\nfn frame() {\n    let _ = Frame;\n}\n",
    );
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    ok(&dir, "person", &["init", "--local"]);
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replace(
            "denylist = []",
            "denylist = [\"example client\"]\ndocs_item = false",
        ) + "\n[map]\ncontracts = [\"docs/*.md\"]\n",
    )
    .unwrap();
    dir
}

/// The phase's work, committed on its branch: `wire.rs` modified,
/// `net.rs` added and declared, `old.rs` deleted, the contract document
/// extended, the README (not on the map) changed, and two test files
/// (left off the map) changed: one modified, one added.
fn work(dir: &Path) {
    git(dir, &["switch", "-q", "phase/P-1"]);
    write(dir, "src/wire.rs", "pub struct Frame;\n// two\n// three\n");
    write(
        dir,
        "src/net.rs",
        "use std::net::TcpStream;\n\nuse super::wire::Frame;\n\npub fn send(_: Frame, _: TcpStream) {}\n",
    );
    write(
        dir,
        "src/lib.rs",
        "pub mod net;\npub mod store;\npub mod wire;\n",
    );
    fs::remove_file(dir.join("src/old.rs")).unwrap();
    write(
        dir,
        "docs/wire.md",
        "# The wire format\n\nOne frame.\nSent over TCP.\n",
    );
    write(dir, "README.md", "a fixture, changed\n");
    write(
        dir,
        "tests/wire.rs",
        "use fixture::wire::Frame;\n\n#[test]\nfn frame() {\n    let _ = Frame;\n    let _ = Frame;\n}\n",
    );
    write(
        dir,
        "tests/net.rs",
        "use fixture::net::send;\n\n#[test]\nfn sends() {}\n",
    );
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "feat: send frames"]);
    git(dir, &["switch", "-q", "main"]);
}

fn node<'a>(doc: &'a Value, path: &str) -> &'a Value {
    doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == path)
        .unwrap_or_else(|| panic!("no node {path} in {doc}"))
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

#[test]
fn the_map_is_the_source_files_with_the_phase_diff_and_how_they_connect() {
    let dir = setup("document");
    ok(&dir, "person", &["phase", "open", "send frames"]);
    let main = git(&dir, &["rev-parse", "main"]);

    // Before any work the phase's tip is where it left the main line:
    // every file is there, and none has changed.
    let before = data(&dir, &["map"]);
    assert_eq!(before["base"], json!(main), "{before}");
    assert_eq!(before["tip"], json!(main), "{before}");
    assert_eq!(before["totals"]["changed"], 0, "{before}");
    assert_eq!(node(&before, "src/old.rs")["status"], "unchanged");
    assert_eq!(before["diffs"], json!({}), "nothing changed: {before}");

    work(&dir);
    let tip = git(&dir, &["rev-parse", "phase/P-1"]);
    let doc = data(&dir, &["map"]);

    // The document's keys and a node's, pinned: within a format they
    // are added to, never removed or renamed.
    assert_eq!(
        keys(&doc),
        [
            "base", "diffs", "edges", "nodes", "phase", "tip", "tip_ref", "totals"
        ]
    );
    assert_eq!(
        keys(&doc["nodes"][0]),
        [
            "added", "contract", "external", "language", "path", "removed", "status", "weight"
        ]
    );
    assert_eq!(keys(&doc["edges"][0]), ["from", "kind", "to"]);
    assert_eq!(
        keys(&doc["totals"]),
        ["added", "changed", "files", "outside", "removed", "tests"]
    );
    assert_eq!(
        keys(&doc["totals"]["tests"]),
        ["added", "changed", "files", "removed"]
    );

    // The phase branch against where it left the main line, read from
    // git: the checkout is on main and has none of the work.
    assert_eq!(doc["phase"], "P-1");
    assert_eq!(doc["base"], json!(main));
    assert_eq!(doc["tip"], json!(tip));
    assert_eq!(doc["tip_ref"], "phase/P-1");
    assert!(!dir.join("src/net.rs").exists());

    // Nodes: the source files and the contract, by path. The README
    // and the manifest are neither; the deleted file is still shown.
    // The test files are source but tests, and are not nodes though
    // the phase changed them.
    let paths: Vec<&str> = doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "docs/wire.md",
            "src/lib.rs",
            "src/net.rs",
            "src/old.rs",
            "src/store.rs",
            "src/wire.rs"
        ]
    );
    let line = |path: &str| {
        let n = node(&doc, path);
        (
            n["status"].as_str().unwrap().to_owned(),
            n["added"].as_u64().unwrap(),
            n["removed"].as_u64().unwrap(),
        )
    };
    assert_eq!(line("src/wire.rs"), ("modified".to_owned(), 2, 1));
    assert_eq!(line("src/net.rs"), ("added".to_owned(), 5, 0));
    assert_eq!(line("src/old.rs"), ("deleted".to_owned(), 0, 1));
    assert_eq!(line("src/lib.rs"), ("modified".to_owned(), 1, 1));
    assert_eq!(line("src/store.rs"), ("unchanged".to_owned(), 0, 0));
    assert_eq!(line("docs/wire.md"), ("modified".to_owned(), 1, 0));
    assert_eq!(
        doc["totals"],
        json!({
            "files": 6, "changed": 5, "added": 9, "removed": 3, "outside": 3,
            // tests/wire.rs +1, tests/net.rs added with 4 lines.
            "tests": { "files": 2, "changed": 2, "added": 5, "removed": 0 }
        })
    );

    // Outside: found from what a file imports, or named as a contract.
    assert_eq!(
        node(&doc, "src/store.rs")["external"],
        json!(["filesystem"])
    );
    assert_eq!(node(&doc, "src/net.rs")["external"], json!(["network"]));
    assert_eq!(node(&doc, "src/wire.rs")["external"], json!([]));
    let contract = node(&doc, "docs/wire.md");
    assert_eq!(contract["contract"], true);
    assert!(contract["language"].is_null(), "{contract}");
    assert_eq!(node(&doc, "src/wire.rs")["language"], "rust");
    assert_eq!(node(&doc, "src/wire.rs")["contract"], false);

    // Weight grows with the diff and with being outside.
    let weight = |path: &str| node(&doc, path)["weight"].as_f64().unwrap();
    assert_eq!(weight("src/store.rs"), 3.0, "unchanged, and outside");
    assert!(weight("src/wire.rs") > 1.0 && weight("src/wire.rs") < weight("src/net.rs"));
    assert!(weight("src/net.rs") > weight("src/store.rs"));

    // Edges: mod lines and use lines, from the file that names the
    // other. The deleted file has none.
    let edges: Vec<(&str, &str, &str)> = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap(),
                e["kind"].as_str().unwrap(),
                e["to"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        [
            ("src/lib.rs", "mod", "src/net.rs"),
            ("src/lib.rs", "mod", "src/store.rs"),
            ("src/lib.rs", "mod", "src/wire.rs"),
            ("src/net.rs", "use", "src/wire.rs"),
            ("src/store.rs", "use", "src/wire.rs"),
        ]
    );
    assert!(
        !doc.to_string().contains("tests/"),
        "no node, edge or diff names a test file: {doc}"
    );

    // Diffs: each changed node's unified diff between base and tip, by
    // path. An unchanged file has none, and neither has a file that is
    // not a node, whatever the phase did to it.
    assert_eq!(
        keys(&doc["diffs"]),
        [
            "docs/wire.md",
            "src/lib.rs",
            "src/net.rs",
            "src/old.rs",
            "src/wire.rs"
        ]
    );
    let diff = |path: &str| doc["diffs"][path].as_str().unwrap().to_owned();
    // The whole text of one, less the line that carries blob ids: the
    // file header, the hunk header, then context, removed and added
    // lines, each marked in its first column.
    let wire = diff("src/wire.rs");
    let lines: Vec<&str> = wire.lines().filter(|l| !l.starts_with("index ")).collect();
    assert_eq!(
        lines,
        [
            "diff --git a/src/wire.rs b/src/wire.rs",
            "--- a/src/wire.rs",
            "+++ b/src/wire.rs",
            "@@ -1,2 +1,3 @@",
            " pub struct Frame;",
            "-// one",
            "+// two",
            "+// three",
        ]
    );
    assert!(wire.ends_with("+// three\n"), "{wire}");
    // An added file is all additions, a deleted one all removals.
    let net = diff("src/net.rs");
    assert!(
        net.contains(
            "\n--- /dev/null\n+++ b/src/net.rs\n@@ -0,0 +1,5 @@\n+use std::net::TcpStream;\n"
        ),
        "{net}"
    );
    let old = diff("src/old.rs");
    assert!(
        old.ends_with("\n--- a/src/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-pub fn gone() {}\n"),
        "{old}"
    );
    // A diff's marked lines are the node's numbers.
    for n in doc["nodes"].as_array().unwrap() {
        let path = n["path"].as_str().unwrap();
        let Some(text) = doc["diffs"][path].as_str() else {
            assert_eq!(n["status"], "unchanged", "{path} has no diff");
            continue;
        };
        // Past the file header and the one hunk header these small
        // files have, whose own text begins with the old range.
        let hunk = text.split_once("\n@@ ").expect("a hunk").1;
        let body = hunk.split_once('\n').expect("the hunk's lines").1;
        let count = |mark: char| body.lines().filter(|l| l.starts_with(mark)).count() as u64;
        assert_eq!(count('+'), n["added"].as_u64().unwrap(), "{path}");
        assert_eq!(count('-'), n["removed"].as_u64().unwrap(), "{path}");
    }

    // In text: the totals, then the changed files with their numbers.
    let text = ok(&dir, "person", &["map"]);
    assert!(
        text.starts_with("P-1: 5 of 6 source file(s) changed (+9 -3), 3 touch the outside"),
        "{text}"
    );
    assert!(
        text.contains("\n2 test file(s) left off the map: 2 changed (+5 -0)\n"),
        "{text}"
    );
    assert!(!text.contains("tests/"), "{text}");
    assert!(text.contains("src/net.rs (added)  [network]"), "{text}");
    assert!(text.contains("docs/wire.md  [contract]"), "{text}");
    assert!(!text.contains("src/store.rs"), "unchanged: {text}");
    assert!(text.contains("`ris map --html map.html`"), "{text}");

    // A seat reads the same map from its own place.
    let seat: Value = serde_json::from_str(&ok(&dir, "heron", &["--json", "map"])).unwrap();
    assert_eq!(seat["data"], doc);
}

#[test]
fn the_page_embeds_the_document_and_a_closed_phase_keeps_its_map() {
    let dir = setup("page");
    ok(&dir, "person", &["phase", "open", "send frames"]);
    work(&dir);
    let doc = data(&dir, &["map"]);

    // One self-contained page: the document whole, d3 inlined, and
    // nothing fetched.
    let page = dir.join(".osiris/map.html");
    let out = ok(&dir, "person", &["map", "--html", page.to_str().unwrap()]);
    assert!(out.contains("wrote "), "{out}");
    let html = fs::read_to_string(&page).unwrap();
    let open = "<script id=\"map-data\" type=\"application/json\">";
    let start = html.find(open).expect("the data element") + open.len();
    let end = start + html[start..].find("</script>").unwrap();
    let embedded: Value = serde_json::from_str(&html[start..end]).unwrap();
    assert_eq!(embedded, doc);
    // The diffs are in the embedded document, so the pane that shows
    // one draws it from the page alone.
    assert!(
        embedded["diffs"]["src/wire.rs"]
            .as_str()
            .is_some_and(|d| d.contains("\n-// one\n+// two\n")),
        "{embedded}"
    );
    assert!(html.contains("<aside id=\"pane\""), "the diff pane");
    assert!(html.contains("doc.diffs"), "the pane reads the document");
    assert!(html.contains("forceSimulation"), "d3 is inlined");
    assert!(html.contains("d3js.org v7"), "the vendored build's banner");
    // No element that loads anything: every script is inline, and
    // there is no stylesheet link, image, frame or import.
    assert_eq!(
        html.matches("<script").count(),
        3,
        "data, d3, the page's own"
    );
    for fetched in ["<script src", "<link", "<img", "<iframe", "@import"] {
        assert!(
            !html.contains(fetched),
            "the page fetches nothing: {fetched}"
        );
    }

    // Closing names the command, and the closed phase's map is read
    // from its closing merge: the same files with the same numbers.
    let closed = ok(&dir, "person", &["phase", "close"]);
    assert!(closed.contains("`ris map --html <file>`"), "{closed}");
    assert!(
        git(&dir, &["branch", "--list", "phase/P-1"]).is_empty(),
        "the phase branch is gone"
    );
    let after = data(&dir, &["map"]);
    assert_eq!(after["phase"], "P-1");
    assert_eq!(after["tip_ref"], "the closing merge of P-1 on main");
    assert_eq!(after["tip"], json!(git(&dir, &["rev-parse", "main"])));
    assert_eq!(after["base"], doc["base"]);
    assert_eq!(after["nodes"], doc["nodes"]);
    assert_eq!(after["edges"], doc["edges"]);
    assert_eq!(after["diffs"], doc["diffs"]);
    assert_eq!(data(&dir, &["map", "P-1"]), after);

    // A phase that does not exist is said to be so.
    let none = ris(&dir, "person", &["map", "P-7"]);
    assert_eq!(none.status.code(), Some(1), "{none:?}");
    assert!(
        String::from_utf8_lossy(&none.stderr).contains("no phase P-7"),
        "{none:?}"
    );
}
