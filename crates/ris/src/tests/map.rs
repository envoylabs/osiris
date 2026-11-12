use super::*;

fn tree(files: &[(&str, &str)]) -> Tree {
    files
        .iter()
        .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
        .collect()
}

fn edges(scans: &BTreeMap<String, Scan>, from: &str) -> Vec<(String, &'static str)> {
    scans[from].edges.iter().cloned().collect()
}

#[test]
fn source_is_named_by_extension_and_a_minified_build_is_not_source() {
    assert_eq!(language_of("crates/ris/src/map.rs"), Some("rust"));
    assert_eq!(language_of("web/app.ts"), Some("typescript"));
    assert_eq!(language_of("README.md"), None);
    assert_eq!(language_of("Makefile"), None);
    assert_eq!(language_of("assets/d3.v7.min.js"), None);
}

#[test]
fn a_glob_matches_within_segments_and_across_them() {
    assert!(glob("docs/json.md", "docs/json.md"));
    assert!(glob(
        "crates/*/src/event.rs",
        "crates/ris-core/src/event.rs"
    ));
    assert!(!glob("crates/*/event.rs", "crates/ris-core/src/event.rs"));
    assert!(glob("**/*.proto", "api/v1/log.proto"));
    assert!(glob("**/*.proto", "log.proto"));
    assert!(glob("schema/**", "schema/a/b.sql"));
    assert!(!glob("*.rs", "src/lib.rs"));
}

#[test]
fn weight_grows_with_the_diff_and_with_touching_the_outside() {
    assert_eq!(weight(0, 0, false), 1.0);
    assert!(weight(10, 0, false) > weight(1, 0, false));
    assert!(weight(0, 10, false) > weight(0, 1, false));
    assert!(weight(500, 0, false) > weight(50, 0, false));
    assert_eq!(weight(3, 4, true), weight(3, 4, false) + 2.0);
}

#[test]
fn the_page_sizes_a_node_by_its_diff_alone_and_fits_the_graph() {
    // The document's weight mixes in touching the outside, which the
    // page shows as a ring; a page that sized nodes by it again would
    // make an unchanged contract look like a large change.
    assert!(!PAGE.contains(".weight"), "the page reads no weight");
    assert!(PAGE.contains("n.added + n.removed"), "size is the diff");
    assert!(PAGE.contains("Math.log1p(most)"), "scaled to the largest");
    assert!(PAGE.contains("size: lines added and removed"), "the legend");
    // The layout is settled before the first frame and then fitted.
    assert!(PAGE.contains("zoom.transform"), "fitted to the window");
    assert!(PAGE.contains("sim.tick()"), "settled before drawing");
}

#[test]
fn test_files_are_counted_and_left_off_unless_a_contract_names_them() {
    let range = Range {
        phase: None,
        base: Some("base".to_owned()),
        tip: "tip".to_owned(),
        tip_ref: "HEAD".to_owned(),
    };
    let paths: Vec<String> = [
        "src/lib.rs",
        "src/lib_test.rs",
        "crates/a/tests/it.rs",
        "tests/fixtures/data.txt",
        "tests/schema.proto",
    ]
    .map(str::to_owned)
    .to_vec();
    let t = tree(&[
        ("src/lib.rs", "mod lib_test;\n"),
        ("src/lib_test.rs", ""),
        ("crates/a/tests/it.rs", ""),
        ("tests/schema.proto", ""),
    ]);
    let diff: BTreeMap<String, (&'static str, u64, u64)> = [
        ("src/lib_test.rs".to_owned(), ("added", 4, 0)),
        ("crates/a/tests/it.rs".to_owned(), ("modified", 2, 3)),
        ("tests/fixtures/data.txt".to_owned(), ("modified", 9, 9)),
    ]
    .into_iter()
    .collect();
    let paths_of = |m: &crate::schema::Map| -> Vec<String> {
        m.nodes.iter().map(|n| n.path.clone()).collect()
    };

    // The default: under a tests directory, at any depth. A file
    // that is not source is not counted as a test either.
    let config = crate::config::MapConfig::default();
    let map = document(&range, &paths, &t, &diff, &config);
    assert_eq!(paths_of(&map), ["src/lib.rs", "src/lib_test.rs"]);
    let tests = &map.totals.tests;
    assert_eq!((tests.files, tests.changed), (2, 1));
    assert_eq!((tests.added, tests.removed), (2, 3));
    assert_eq!(map.edges.len(), 1, "src/lib.rs to src/lib_test.rs");

    // A pattern of the project's own, and a contract under tests
    // stays on the map: the person named it.
    let config = crate::config::MapConfig {
        contracts: vec!["tests/*.proto".to_owned()],
        tests: vec!["**/tests/**".to_owned(), "**/*_test.rs".to_owned()],
    };
    let map = document(&range, &paths, &t, &diff, &config);
    assert_eq!(paths_of(&map), ["src/lib.rs", "tests/schema.proto"]);
    assert!(map.edges.is_empty(), "no edge to a test file");
    let tests = &map.totals.tests;
    assert_eq!((tests.files, tests.changed), (2, 2));
    assert_eq!((tests.added, tests.removed), (6, 3));
    assert_eq!(
        tests_line(tests),
        "2 test file(s) left off the map: 2 changed (+6 -3)"
    );
}

#[test]
fn the_page_writes_the_tests_line_from_the_document() {
    assert!(PAGE.contains("id=\"tests\""));
    assert!(PAGE.contains("t.tests.files + \" test file(s) left off the map: \""));
}

#[test]
fn comments_strings_and_character_literals_name_nothing() {
    let found = names(
        r##"
            // use crate::in_a_comment;
            /* mod blocked; /* nested */ use crate::still_blocked; */
            const A: &str = "use crate::in_a_string; \" mod quoted;";
            const B: &str = r#"use crate::in_a_raw_string;"#;
            const C: char = '"';
            const D: char = '\'';
            fn f<'a>(x: &'a str) -> &'a str { crate::real::call(x) }
            mod r#type;
            "##,
    );
    assert_eq!(found.mods, ["type"]);
    assert_eq!(
        found.paths,
        [(
            vec!["crate".to_owned(), "real".to_owned(), "call".to_owned()],
            0
        )]
    );
}

#[test]
fn a_use_tree_gives_every_leaf_and_an_inline_module_is_counted() {
    let found = names(
        "pub use crate::{a::B, c::{self, D as E}, f::*};\n\
             mod inner { use super::g; mod deeper { use super::super::h; } }\n\
             use std::fs;",
    );
    let paths: Vec<(String, usize)> = found
        .paths
        .iter()
        .map(|(p, depth)| (p.join("::"), *depth))
        .collect();
    assert_eq!(
        paths,
        [
            ("crate::a::B".to_owned(), 0),
            ("crate::c::self".to_owned(), 0),
            ("crate::c::D".to_owned(), 0),
            ("crate::f".to_owned(), 0),
            ("super::g".to_owned(), 1),
            ("super::super::h".to_owned(), 2),
            ("std::fs".to_owned(), 0),
        ]
    );
    assert!(found.mods.is_empty(), "inline modules are not files");
}

#[test]
fn rust_edges_follow_mod_lines_use_lines_and_paths_across_packages() {
    let t = tree(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"core\", \"cli\"]\n"),
        ("core/Cargo.toml", "[package]\nname = \"my-core\"\n"),
        ("core/src/lib.rs", "pub mod event;\npub mod store;\n"),
        ("core/src/event.rs", "use super::store::Store;\n"),
        ("core/src/store/mod.rs", "mod disk;\n"),
        (
            "core/src/store/disk.rs",
            "use std::fs;\nuse crate::event::Event;\n\
                 #[cfg(test)]\nmod tests { use super::*; }\n",
        ),
        ("cli/Cargo.toml", "[package]\nname = \"cli\"\n"),
        (
            "cli/src/main.rs",
            "mod run;\nfn main() { my_core::event::go(); my_core::top(); std::process::exit(0) }\n",
        ),
        (
            "cli/src/run.rs",
            "use std::process::Command;\nuse my_core::store::disk::Disk;\n",
        ),
        (
            "cli/tests/it.rs",
            "mod common;\nuse cli::run;\nuse std::net::TcpStream;\nuse rusqlite::Connection;\n",
        ),
        ("cli/tests/common/mod.rs", "pub fn setup() {}\n"),
    ]);
    let scans = Rust.scan(&t);
    assert_eq!(
        edges(&scans, "core/src/lib.rs"),
        [
            ("core/src/event.rs".to_owned(), "mod"),
            ("core/src/store/mod.rs".to_owned(), "mod"),
        ]
    );
    assert_eq!(
        edges(&scans, "core/src/event.rs"),
        [("core/src/store/mod.rs".to_owned(), "use")]
    );
    assert_eq!(
        edges(&scans, "core/src/store/mod.rs"),
        [("core/src/store/disk.rs".to_owned(), "mod")]
    );
    // `super::*` in the file's own test module is the file itself:
    // no edge to the module above.
    assert_eq!(
        edges(&scans, "core/src/store/disk.rs"),
        [("core/src/event.rs".to_owned(), "use")]
    );
    assert_eq!(
        edges(&scans, "cli/src/main.rs"),
        [
            ("cli/src/run.rs".to_owned(), "mod"),
            ("core/src/event.rs".to_owned(), "use"),
            ("core/src/lib.rs".to_owned(), "use"),
        ]
    );
    assert_eq!(
        edges(&scans, "cli/src/run.rs"),
        [("core/src/store/disk.rs".to_owned(), "use")]
    );
    // A test file is its own crate: its `mod` is beside it, and the
    // package's name reaches the package's source.
    assert_eq!(
        edges(&scans, "cli/tests/it.rs"),
        [
            ("cli/src/run.rs".to_owned(), "use"),
            ("cli/tests/common/mod.rs".to_owned(), "mod"),
        ]
    );
    let outside = |f: &str| scans[f].external.iter().copied().collect::<Vec<_>>();
    assert_eq!(outside("core/src/store/disk.rs"), ["filesystem"]);
    assert_eq!(outside("cli/src/run.rs"), ["process"]);
    assert_eq!(outside("cli/tests/it.rs"), ["database", "network"]);
    assert!(outside("core/src/event.rs").is_empty());
    // Ending the process is not starting one.
    assert!(outside("cli/src/main.rs").is_empty());
}

#[test]
fn the_page_embeds_the_document_and_the_vendored_build_cannot_end_its_script() {
    assert!(
        !D3.contains("</script"),
        "the d3 build would end its element"
    );
    let doc = serde_json::json!({ "nodes": [{ "path": "a</script><b>.rs" }], "edges": [] });
    let html = page(&doc);
    assert!(!html.contains("/*D3*/") && !html.contains("/*DATA*/"));
    assert!(!html.contains("a</script>"), "the document is escaped");
    let open = "<script id=\"map-data\" type=\"application/json\">";
    let start = html.find(open).expect("the data element") + open.len();
    let end = start + html[start..].find("</script>").unwrap();
    let back: serde_json::Value = serde_json::from_str(&html[start..end]).unwrap();
    assert_eq!(back, doc);
}
