//! `ris map` over a project that is not Rust: a small fixture in each
//! of TypeScript, Go and Clojure, kept in `tests/fixtures/map`, is
//! committed in two steps and mapped over the git range between them.
//! Each file connects to the files it imports, the files that import it
//! are counted, its test files are left off by the default patterns, and
//! the range is read in any repository, a project or not.

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

/// `ris` as anyone at all: outside a project no seat is needed.
fn ris(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = ris(dir, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn map(dir: &Path, range: &str) -> Value {
    let out = ok(dir, &["--json", "map", range]);
    common::validate_output(&["map"], &out);
    serde_json::from_str::<Value>(&out).unwrap()["data"].clone()
}

fn copy(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// The fixture for `language` in a repository of its own: every file
/// but `later` committed first, then `later` in a second commit.
fn repository(name: &str, language: &str, later: &[&str]) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/map")
        .join(language);
    let dir = std::env::temp_dir().join(format!("ris-map-languages-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Named in its canonical form: the temporary directory may be
    // reached through a link.
    let dir = dir.canonicalize().unwrap();
    copy(&fixture, &dir);
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "-A"]);
    for path in later {
        git(&dir, &["rm", "-q", "--cached", path]);
    }
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "the change"]);
    dir
}

fn edges(doc: &Value) -> Vec<(String, String, String)> {
    doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn edge(from: &str, to: &str, kind: &str) -> (String, String, String) {
    (from.to_owned(), to.to_owned(), kind.to_owned())
}

fn node<'a>(doc: &'a Value, path: &str) -> &'a Value {
    doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == path)
        .unwrap_or_else(|| panic!("no node {path} in {doc}"))
}

fn paths(doc: &Value) -> Vec<&str> {
    doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap())
        .collect()
}

#[test]
fn typescript_files_connect_through_their_imports() {
    let dir = repository("ts", "typescript", &["src/net.ts"]);
    let doc = map(&dir, "HEAD~1..HEAD");
    assert_eq!(doc["phase"], Value::Null);
    assert_eq!(doc["tip_ref"], "HEAD~1..HEAD");
    assert_eq!(doc["base"], json!(git(&dir, &["rev-parse", "HEAD~1"])));
    assert_eq!(doc["tip"], json!(git(&dir, &["rev-parse", "HEAD"])));
    assert_eq!(
        edges(&doc),
        vec![
            edge("src/index.ts", "src/store.ts", "import"),
            edge("src/index.ts", "src/ui/index.tsx", "import"),
            edge("src/index.ts", "src/wire.ts", "import"),
            edge("src/net.ts", "src/wire.ts", "import"),
            edge("src/store.ts", "src/wire.ts", "import"),
        ]
    );
    assert_eq!(node(&doc, "src/wire.ts")["dependents"], 3);
    assert_eq!(
        node(&doc, "src/store.ts")["external"],
        json!(["filesystem"])
    );
    let net = node(&doc, "src/net.ts");
    assert_eq!(net["status"], "added");
    assert_eq!(net["external"], json!(["network"]));
    // The test beside the source is left off, and counted.
    assert!(!paths(&doc).contains(&"src/store.test.ts"), "{doc}");
    assert_eq!(doc["totals"]["tests"]["files"], 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn go_files_connect_to_the_packages_they_import() {
    let dir = repository("go", "go", &["wire/wire.go", "cmd/tool/tool.go"]);
    let doc = map(&dir, "HEAD~1..HEAD");
    assert_eq!(
        edges(&doc),
        vec![
            edge("cmd/tool/tool.go", "wire/wire.go", "import"),
            edge("main.go", "store/cache.go", "import"),
            edge("main.go", "store/store.go", "import"),
        ]
    );
    assert_eq!(node(&doc, "wire/wire.go")["dependents"], 1);
    assert_eq!(node(&doc, "wire/wire.go")["external"], json!(["network"]));
    assert_eq!(
        node(&doc, "store/store.go")["external"],
        json!(["filesystem"])
    );
    assert!(!paths(&doc).contains(&"store/store_test.go"), "{doc}");
    assert_eq!(doc["totals"]["tests"]["files"], 1);
    assert_eq!(doc["totals"]["changed"], 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn clojure_namespaces_connect_through_their_requires() {
    let dir = repository("clj", "clojure", &["src/shop/net.clj"]);
    let doc = map(&dir, "HEAD~1..HEAD");
    assert_eq!(
        edges(&doc),
        vec![
            edge("src/shop/core.clj", "src/shop/store.clj", "require"),
            edge("src/shop/core.clj", "src/shop/wire.cljc", "require"),
            edge("src/shop/net.clj", "src/shop/wire.cljc", "require"),
        ]
    );
    assert_eq!(node(&doc, "src/shop/wire.cljc")["dependents"], 2);
    assert_eq!(
        node(&doc, "src/shop/store.clj")["external"],
        json!(["filesystem"])
    );
    assert_eq!(
        node(&doc, "src/shop/net.clj")["external"],
        json!(["network"])
    );
    // Clojure's tests live under test/, left off by default.
    assert!(!paths(&doc).contains(&"test/shop/store_test.clj"), "{doc}");
    assert_eq!(doc["totals"]["tests"]["files"], 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_range_is_read_in_a_project_and_out_of_one() {
    let dir = repository("range", "typescript", &["src/net.ts"]);
    // Out of a project: the text names the range, and `A...B` is the
    // same here, where HEAD~1 is where the two met.
    let text = ok(&dir, &["map", "HEAD~1..HEAD"]);
    assert!(
        text.starts_with("HEAD~1..HEAD: 1 of 5 source file(s) changed (+6 -0)"),
        "{text}"
    );
    assert_eq!(
        map(&dir, "HEAD~1...HEAD")["nodes"],
        map(&dir, "HEAD~1..HEAD")["nodes"]
    );
    let list = ok(&dir, &["map", "HEAD~1..", "--list"]);
    assert!(list.contains("src/net.ts (added)  [network]"), "{list}");
    let page = dir.join("map.html");
    ok(
        &dir,
        &["map", "HEAD~1..HEAD", "--html", page.to_str().unwrap()],
    );
    assert!(fs::read_to_string(&page).unwrap().contains("map-data"));
    fs::remove_file(&page).unwrap();

    // What is not a range or a commit is said to be so.
    let bad = ris(&dir, &["map", "nowhere..HEAD"]);
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("`nowhere` is not a commit"),
        "{bad:?}"
    );
    let bad = ris(&dir, &["map", "banana"]);
    assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("neither a phase (P-n) nor a git range"),
        "{bad:?}"
    );

    // In a project, as the person: the same document, with no phase.
    let out = ris(&dir, &["init", "--local"]);
    assert!(out.status.success(), "{out:?}");
    let inside = Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(["--json", "map", "HEAD~1..HEAD"])
        .current_dir(&dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .output()
        .unwrap();
    assert!(inside.status.success(), "{inside:?}");
    let inside: Value = serde_json::from_slice(&inside.stdout).unwrap();
    assert_eq!(inside["data"]["phase"], Value::Null);
    assert_eq!(inside["data"]["edges"], map_outside(&dir)["edges"]);
    let _ = fs::remove_dir_all(&dir);
}

/// The range's document with the project hidden from `ris`: read from a
/// copy of the repository without `.osiris`.
fn map_outside(dir: &Path) -> Value {
    let bare = dir.with_extension("bare");
    let _ = fs::remove_dir_all(&bare);
    git(
        dir.parent().unwrap(),
        &["clone", "-q", dir.to_str().unwrap(), bare.to_str().unwrap()],
    );
    let doc = map(&bare, "HEAD~1..HEAD");
    let _ = fs::remove_dir_all(&bare);
    doc
}
