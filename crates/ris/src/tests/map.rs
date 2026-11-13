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
    let map = document(&range, &paths, &t, &diff, &config, &BTreeMap::new());
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
    let map = document(&range, &paths, &t, &diff, &config, &BTreeMap::new());
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
fn the_computed_level_adds_up_the_signals_and_lists_those_that_count() {
    let signals = |c: &Computed| c.signals.iter().map(|s| s.signal).collect::<Vec<_>>();
    // Nothing counts: low, with no signal listed.
    let quiet = computed_risk(&[], false, 49, 2);
    assert_eq!((quiet.level, signals(&quiet)), (RiskLevel::Low, vec![]));
    // One mild signal alone stays low, and is still listed.
    let outside = computed_risk(&["filesystem", "process"], false, 3, 0);
    assert_eq!(outside.level, RiskLevel::Low);
    assert_eq!(
        outside.signals,
        [Signal {
            signal: "outside",
            reason: "touches the outside: filesystem, process".to_owned()
        }]
    );
    assert_eq!(computed_risk(&[], false, 50, 0).level, RiskLevel::Low);
    assert_eq!(computed_risk(&[], false, 0, 3).level, RiskLevel::Low);
    // A contract, a large change or many dependents alone is medium.
    assert_eq!(computed_risk(&[], true, 1, 0).level, RiskLevel::Medium);
    assert_eq!(computed_risk(&[], false, 200, 0).level, RiskLevel::Medium);
    assert_eq!(computed_risk(&[], false, 0, 8).level, RiskLevel::Medium);
    // So are two mild signals together.
    assert_eq!(
        computed_risk(&["network"], false, 50, 0).level,
        RiskLevel::Medium
    );
    assert_eq!(
        computed_risk(&["network"], true, 1, 0).level,
        RiskLevel::Medium
    );
    // High takes signals together.
    assert_eq!(computed_risk(&[], true, 200, 0).level, RiskLevel::High);
    assert_eq!(computed_risk(&[], false, 200, 8).level, RiskLevel::High);
    assert_eq!(
        computed_risk(&["network"], true, 50, 0).level,
        RiskLevel::High
    );
    let all = computed_risk(&["database"], true, 340, 12);
    assert_eq!(all.level, RiskLevel::High);
    assert_eq!(signals(&all), ["outside", "contract", "diff", "dependents"]);
    assert_eq!(all.signals[2].reason, "a large change: 340 lines");
    assert_eq!(all.signals[3].reason, "12 other files depend on it");
    // More of anything never lowers the level.
    for lines in [0, 49, 50, 199, 200, 5000] {
        for dependents in [0, 2, 3, 7, 8, 40] {
            let here = computed_risk(&[], false, lines, dependents).level;
            assert!(computed_risk(&["process"], false, lines, dependents).level >= here);
            assert!(computed_risk(&[], true, lines, dependents).level >= here);
            assert!(computed_risk(&[], false, lines + 200, dependents).level >= here);
            assert!(computed_risk(&[], false, lines, dependents + 8).level >= here);
        }
    }
}

#[test]
fn a_changed_file_carries_its_risk_and_a_judgement_stands_over_the_computed_level() {
    let range = Range {
        phase: None,
        base: Some("base".to_owned()),
        tip: "tip".to_owned(),
        tip_ref: "HEAD".to_owned(),
    };
    let paths: Vec<String> = [
        "src/a.rs",
        "src/b.rs",
        "src/c.rs",
        "src/lib.rs",
        "src/wire.rs",
    ]
    .map(str::to_owned)
    .to_vec();
    let t = tree(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        ("src/lib.rs", "mod a;\nmod b;\nmod c;\nmod wire;\n"),
        ("src/a.rs", "use crate::wire::Frame;\n"),
        // Named by a `use` line and by a path: one dependent, once.
        (
            "src/b.rs",
            "use crate::wire::Frame;\nfn f() { crate::wire::go() }\n",
        ),
        ("src/c.rs", "use crate::wire::Frame;\nuse std::fs;\n"),
        ("src/wire.rs", "pub struct Frame;\n"),
    ]);
    let diff: BTreeMap<String, (&'static str, u64, u64)> = [
        ("src/wire.rs".to_owned(), ("modified", 40, 20)),
        ("src/c.rs".to_owned(), ("modified", 2, 0)),
        ("src/old.rs".to_owned(), ("deleted", 0, 9)),
    ]
    .into_iter()
    .collect();
    let config = crate::config::MapConfig::default();
    let judged: BTreeMap<String, Judged> = [
        (
            "src/c.rs".to_owned(),
            Judged {
                level: RiskLevel::High,
                why: "writes outside the store's directory".to_owned(),
                seat: "wren".to_owned(),
            },
        ),
        // One for a file the phase did not change is not carried.
        (
            "src/a.rs".to_owned(),
            Judged {
                level: RiskLevel::High,
                why: "stale".to_owned(),
                seat: "wren".to_owned(),
            },
        ),
    ]
    .into_iter()
    .collect();
    let map = document(&range, &paths, &t, &diff, &config, &judged);
    let node = |path: &str| map.nodes.iter().find(|n| n.path == path).unwrap();

    // Dependents: the files with an edge to it, the declaring module
    // among them.
    assert_eq!(node("src/wire.rs").dependents, 4, "lib, a, b and c");
    assert_eq!(node("src/c.rs").dependents, 1, "lib");
    assert_eq!(node("src/lib.rs").dependents, 0);

    // Computed alone: a sizeable change that others depend on.
    let wire = node("src/wire.rs").risk.as_ref().unwrap();
    assert_eq!(wire.level, RiskLevel::Medium);
    assert_eq!(wire.computed.level, RiskLevel::Medium);
    assert_eq!(wire.judged, None);
    assert_eq!(
        wire.computed
            .signals
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>(),
        ["a sizeable change: 60 lines", "4 other files depend on it"]
    );

    // Judged: the judgement's level stands, beside the computed one.
    let c = node("src/c.rs").risk.as_ref().unwrap();
    assert_eq!(c.level, RiskLevel::High);
    assert_eq!(c.computed.level, RiskLevel::Low);
    assert_eq!(c.judged, Some(judged["src/c.rs"].clone()));

    // A deleted file is a change and has a level; an unchanged file
    // has none, whatever was recorded for it.
    assert_eq!(
        node("src/old.rs").risk.as_ref().unwrap().level,
        RiskLevel::Low
    );
    assert_eq!(node("src/a.rs").risk, None);
    assert_eq!(node("src/lib.rs").risk, None);

    // As the document spells them.
    let doc = serde_json::to_value(&map).unwrap();
    let c = doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == "src/c.rs")
        .unwrap();
    assert_eq!(
        c["risk"],
        serde_json::json!({
            "level": "high",
            "computed": {
                "level": "low",
                "signals": [
                    { "signal": "outside", "reason": "touches the outside: filesystem" }
                ]
            },
            "judged": {
                "level": "high",
                "why": "writes outside the store's directory",
                "seat": "wren"
            }
        })
    );
}

#[test]
fn the_page_marks_risk_apart_from_size_and_the_ring_and_says_why() {
    // The mark is its own shape, drawn from the level that stands.
    assert!(PAGE.contains("n.risk.level"), "the level is the document's");
    assert!(PAGE.contains("\"mark \" + level(n)"), "a mark per level");
    assert!(PAGE.contains(".node .mark.high { fill: var(--risk); }"));
    // Size is still the diff alone and the ring still the outside.
    assert!(PAGE.contains(".node.outside circle { stroke: var(--outside)"));
    // The reasons, on hover and in the pane's head, from one function.
    assert!(PAGE.contains("riskLines(n).forEach(function (r) { line(r.cls, r.text); })"));
    assert!(PAGE.contains("id=\"pane-risk\""));
    assert!(PAGE.contains("\"judged \" + j.level + \" by \" + j.seat"));
    assert!(PAGE.contains("c.signals.map"));
    // The legend explains it.
    assert!(PAGE.contains("high risk: read this change first"));
    assert!(PAGE.contains("an auditor's judgement where one is recorded"));
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
