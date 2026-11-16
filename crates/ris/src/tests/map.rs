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
        effects: BTreeMap::new(),
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

    // Ranked by the level that stands: the judged file leads the
    // larger change, and unchanged files are not ranked.
    assert_eq!(map.ranked, ["src/c.rs", "src/wire.rs", "src/old.rs"]);

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
fn ranked_is_the_level_then_the_weight_then_the_path() {
    let at = |path: &str, status: &'static str, level: RiskLevel, weight: f64| Node {
        path: path.to_owned(),
        language: Some("rust"),
        status,
        added: 0,
        removed: 0,
        external: Vec::new(),
        contract: false,
        weight,
        dependents: 0,
        risk: (status != "unchanged").then(|| Risk {
            level,
            computed: Computed {
                level,
                signals: Vec::new(),
            },
            judged: None,
        }),
        functions: None,
        marks: None,
    };
    let nodes = [
        at("a.rs", "modified", RiskLevel::Low, 9.0),
        at("b.rs", "modified", RiskLevel::Medium, 1.0),
        at("c.rs", "added", RiskLevel::Low, 9.0),
        at("d.rs", "unchanged", RiskLevel::Low, 0.0),
        at("e.rs", "deleted", RiskLevel::High, 0.5),
        at("f.rs", "modified", RiskLevel::Low, 20.0),
    ];
    assert_eq!(
        ranked(&nodes),
        ["e.rs", "b.rs", "f.rs", "a.rs", "c.rs"],
        "high, medium, then the low ones heaviest first and a tie by path"
    );
}

#[test]
fn a_diff_s_changes_are_read_in_the_tip_s_line_numbers() {
    let diff = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,4 +1,5 @@
 fn a() {
-    one();
+    two();
+    three();
 }
@@ -10,3 +11,2 @@ fn b() {
 x
-y
-z
\\ No newline at end of file
";
    let c = changes(diff);
    assert_eq!(c.added, [2, 3].into_iter().collect());
    // One line removed before tip line 2, two before tip line 12.
    assert_eq!(c.removed, [(2, 1), (12, 2)].into_iter().collect());
    // A new file: every line added.
    let c = changes("--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1,2 @@\n+a\n+b\n");
    assert_eq!(c.added, [1, 2].into_iter().collect());
    assert!(c.removed.is_empty());
    assert_eq!(changes(""), Changes::default());
}

#[test]
fn only_changed_functions_are_kept_and_a_mark_takes_its_function_s_level() {
    use crate::syntax::{Function, Parsed, Reference};
    let f = |name: &str, start, end| Function {
        name: name.to_owned(),
        start,
        end,
    };
    let r = |line, name: &str| Reference {
        line,
        name: name.to_owned(),
    };
    let parsed = Parsed {
        functions: vec![
            f("save", 1, 10),
            f("save::inner", 4, 6),
            f("load", 12, 20),
            f("idle", 22, 30),
        ],
        references: vec![
            r(2, "std::fs::write"),
            r(5, "std::net::TcpStream::connect"),
            r(14, "std::fs::read"),
            r(24, "std::fs::read"),
            r(31, "std::fs::remove_file"),
        ],
        ..Parsed::default()
    };
    let changes = Changes {
        // In `inner`, so in `save` too; and a top-level line.
        added: [5, 31].into_iter().collect(),
        // Lines removed from `load`.
        removed: [(15, 60)].into_iter().collect(),
    };
    let patterns = crate::syntax::default_effects("rust");
    let (functions, marks) = inside(&parsed, &changes, &patterns, &|_| (Vec::new(), Vec::new()));
    let summary: Vec<(&str, u64, u64, Vec<&str>, bool)> = functions
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.added,
                c.removed,
                c.effects.clone(),
                c.direct,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("save", 1, 0, vec!["filesystem", "network"], true),
            ("save::inner", 1, 0, vec!["network"], true),
            ("load", 0, 60, vec!["filesystem"], true),
        ],
        "idle is not changed"
    );
    // A large change that touches the outside is medium.
    assert_eq!(functions[2].risk.level, RiskLevel::Medium);
    assert_eq!(functions[0].risk.level, RiskLevel::Low);
    let marks: Vec<(u64, &str, Option<&str>, Option<RiskLevel>)> = marks
        .iter()
        .map(|m| (m.line, m.kind, m.function.as_deref(), m.risk))
        .collect();
    assert_eq!(
        marks,
        [
            // In a changed function, though the line is not changed.
            (2, "filesystem", Some("save"), Some(RiskLevel::Low)),
            // The innermost function holds it.
            (5, "network", Some("save::inner"), Some(RiskLevel::Low)),
            (14, "filesystem", Some("load"), Some(RiskLevel::Medium)),
            // A changed line outside any function.
            (31, "filesystem", None, None),
        ],
        "line 24, in the unchanged idle, is not marked"
    );
}

#[test]
fn a_changed_function_one_step_from_an_effect_triggers_or_feeds_it() {
    let tree = tree(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/net.rs",
            "pub fn send() {\n    std::net::TcpStream::connect(\"x\").unwrap();\n}\n\
             pub fn save() {\n    std::fs::write(\"y\", b\"\").unwrap();\n}\n\
             pub fn to_string() {\n    std::fs::read(\"z\").unwrap();\n}\n",
        ),
        (
            "src/store.rs",
            "use crate::net::send;\n\npub struct Store;\n\nimpl Store {\n    \
             pub fn save(&self) {\n        send();\n    }\n    \
             pub fn new() -> Self {\n        Store\n    }\n}\n",
        ),
        (
            "src/app.rs",
            "use crate::store::Store;\n\npub fn run() {\n    let s = Store::new();\n    \
             s.save();\n    prepare();\n}\n\npub fn prepare() {}\n\n\
             pub fn write() {\n    std::fs::write(\"w\", b\"\").unwrap();\n    prepare();\n    \
             serde_json::to_string();\n}\n",
        ),
    ]);
    let mut files = BTreeMap::new();
    for (path, text) in &tree {
        if let Some(p) = crate::syntax::parse("rust", path, text).filter(|_| path.ends_with(".rs"))
        {
            files.insert(path.clone(), ("rust", p));
        }
    }
    let locals = Locals::of(&tree, &files);
    assert!(locals.rust_packages.contains("fixture"));
    let graph = Graph::new(&files, &locals, &crate::config::MapConfig::default());
    let function = |path: &str, name: &str| {
        files[path]
            .1
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .clone()
    };
    let callee = |path: &str, function: &str| Callee {
        path: path.to_owned(),
        function: function.to_owned(),
    };
    let step = |direction, path: &str, function: &str, effects: &[&'static str]| Step {
        direction,
        path: path.to_owned(),
        function: function.to_owned(),
        effects: effects.to_vec(),
    };

    // It calls a direct function through a `use` line: it triggers.
    let (calls, steps) = graph.reach("src/store.rs", &function("src/store.rs", "Store::save"));
    assert_eq!(calls, [callee("src/net.rs", "send")]);
    assert_eq!(
        steps,
        [step("triggers", "src/net.rs", "send", &["network"])]
    );

    // A method on a value could be either `save`: it counts for each,
    // and the direct one makes a step.
    let (calls, steps) = graph.reach("src/app.rs", &function("src/app.rs", "run"));
    assert_eq!(
        calls,
        [
            callee("src/app.rs", "prepare"),
            callee("src/net.rs", "save"),
            callee("src/store.rs", "Store::new"),
            callee("src/store.rs", "Store::save"),
        ]
    );
    assert_eq!(
        steps,
        [step("triggers", "src/net.rs", "save", &["filesystem"])]
    );

    // A direct function calls it: it feeds the effect.
    let (_, steps) = graph.reach("src/app.rs", &function("src/app.rs", "prepare"));
    assert_eq!(
        steps,
        [step("feeds", "src/app.rs", "write", &["filesystem"])]
    );

    // A call into another crate is not the repository's, though a
    // function of the same name is.
    let (calls, steps) = graph.reach("src/app.rs", &function("src/app.rs", "write"));
    assert_eq!(calls, [callee("src/app.rs", "prepare")]);
    assert!(steps.is_empty());

    // One step adds a point and says why.
    let risk = function_risk(
        &[],
        1,
        &[step("triggers", "src/net.rs", "send", &["network"])],
    );
    assert_eq!(risk.level, RiskLevel::Low);
    assert_eq!(risk.signals[0].signal, "reach");
    assert_eq!(
        risk.signals[0].reason,
        "one step from an effect: triggers network via send"
    );
    let risk = function_risk(
        &["filesystem"],
        1,
        &[step("feeds", "a.rs", "w", &["network"])],
    );
    assert_eq!(
        risk.level,
        RiskLevel::Medium,
        "touching the outside and one step"
    );
}

#[test]
fn whether_a_call_s_module_is_the_repository_s_own() {
    let locals = Locals {
        rust_packages: ["fixture".to_owned()].into_iter().collect(),
        go_dirs: ["store".to_owned()].into_iter().collect(),
        clojure_namespaces: ["app.store".to_owned()].into_iter().collect(),
    };
    for (language, module, local) in [
        ("rust", None, true),
        ("rust", Some("crate::store"), true),
        ("rust", Some("super"), true),
        ("rust", Some("fixture::wire"), true),
        ("rust", Some("std::fs"), false),
        ("rust", Some("serde_json"), false),
        ("typescript", Some("./store"), true),
        ("typescript", Some("@/lib/store"), true),
        ("typescript", Some("fs"), false),
        ("go", Some("example.com/app/store"), true),
        ("go", Some("net/http"), false),
        ("clojure", Some("app.store"), true),
        ("clojure", Some("clojure.string"), false),
    ] {
        assert_eq!(
            locals.is_local(language, module),
            local,
            "{language} {module:?}"
        );
    }
}

#[test]
fn a_project_s_patterns_add_to_the_shipped_ones() {
    let config = crate::config::Config::parse(
        "[map.effects.rust]\ndatabase = [\"crate::db\"]\n\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    let patterns = config.map.effects_for("rust");
    assert!(patterns.contains(&("crate::db".to_owned(), "database")));
    assert!(patterns.contains(&("std::fs".to_owned(), "filesystem")));
    // A language the map does not read inside, or a kind it does not
    // know, is refused.
    for bad in [
        "[map.effects.python]\nprocess = [\"os.system\"]\n",
        "[map.effects.rust]\nclock = [\"std::time\"]\n",
    ] {
        let err = crate::config::Config::parse(bad).unwrap_err();
        assert!(err.contains("[map.effects."), "{err}");
    }
}

#[test]
fn the_page_lists_the_ranked_files_and_opens_on_the_list() {
    // The list reads the document's own order, and a row opens the
    // same pane a node does.
    assert!(PAGE.contains("(doc.ranked || []).forEach"));
    assert!(PAGE.contains("tr.addEventListener(\"click\", function () { pick(n); })"));
    assert!(PAGE.contains("node.on(\"click\""));
    // Its reasons are the pane's, from the same function.
    assert!(PAGE.contains("riskLines(n).slice(1)"));
    // The page opens on the list, with the graph the other view.
    assert!(PAGE.contains("<body class=\"listing\">"));
    assert!(PAGE.contains("id=\"view-list\" type=\"button\" aria-pressed=\"true\""));
    assert!(PAGE.contains("id=\"view-graph\" type=\"button\" aria-pressed=\"false\""));
    // The graph is hidden, not taken out of the layout, so it can be
    // fitted while the list shows.
    assert!(PAGE.contains(
        "body.listing svg, body.listing #legend, body.listing #tip { visibility: hidden; }"
    ));
    // A file with marked lines expands to show them, without picking
    // it: the pane stays as it is.
    assert!(PAGE.contains("var marks = n.marks || [];"));
    assert!(PAGE.contains("event.stopPropagation();\n        detail.hidden = !detail.hidden;"));
    assert!(PAGE.contains("toggle.setAttribute(\"aria-expanded\""));
    // Expanded, it shows each function one step from an effect too.
    assert!(
        PAGE.contains("f.steps.forEach(function (s) { steps.push({ from: f.name, step: s }); });")
    );
    // The open pane narrows the list as it does the graph.
    assert!(PAGE.contains("body.paned #list { right: calc(var(--pane) + 12px); }"));
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
fn only_the_person_at_a_terminal_with_no_flags_has_the_page_opened() {
    let seat = Actor::Agent(ris_core::ids::AgentName("heron".to_owned()));
    assert!(opens_page(&Actor::Person, false, false, true));
    // Not for a seat, whatever else holds.
    assert!(!opens_page(&seat, false, false, true));
    // Not when the output is piped or captured.
    assert!(!opens_page(&Actor::Person, false, false, false));
    // Not when JSON or a file of the person's own was asked for.
    assert!(!opens_page(&Actor::Person, true, false, true));
    assert!(!opens_page(&Actor::Person, false, true, true));
}

#[test]
fn list_prints_the_list_and_never_opens_the_page() {
    let seat = Actor::Agent(ris_core::ids::AgentName("heron".to_owned()));
    // The person at a terminal with no flags gets the page opened, and
    // with --list, at the same terminal, the list and no page.
    assert_eq!(form(&Actor::Person, false, false, false, true), Form::Open);
    assert_eq!(form(&Actor::Person, false, false, true, true), Form::List);
    assert_eq!(form(&Actor::Person, true, false, true, true), Form::List);
    assert_eq!(form(&seat, false, false, true, false), Form::List);
    // --html writes the page whatever else; otherwise the text form.
    assert_eq!(form(&Actor::Person, false, true, false, true), Form::Write);
    assert_eq!(form(&Actor::Person, false, false, false, false), Form::Text);
    assert_eq!(form(&seat, false, false, false, true), Form::Text);
}

#[test]
fn the_platform_opener_is_the_systems_own() {
    let opener = Opener::platform();
    let want = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "cmd"
    } else {
        "xdg-open"
    };
    assert_eq!(opener.program, want);
}

/// An empty map, for what `output` does with the page.
#[cfg(unix)]
fn empty_map() -> crate::schema::Map {
    let range = Range {
        phase: None,
        base: None,
        tip: "tip".to_owned(),
        tip_ref: "HEAD".to_owned(),
    };
    document(
        &range,
        &[],
        &Tree::new(),
        &BTreeMap::new(),
        &crate::config::MapConfig::default(),
        &BTreeMap::new(),
    )
}

#[cfg(unix)]
#[test]
fn the_page_is_written_to_its_file_and_opened_and_a_failure_to_open_is_said_not_raised() {
    let dir = std::env::temp_dir().join(format!("ris-map-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(PAGE_FILE);
    let show = |opener: &Opener| {
        output(
            empty_map(),
            Show::Open {
                file: &file,
                shown: ".osiris/map.html",
                opener,
            },
        )
        .expect("a browser that does not open is not an error")
        .text
    };
    let sh = |script: &str| Opener {
        program: "sh".to_owned(),
        args: vec!["-c".to_owned(), script.to_owned(), "sh".to_owned()],
    };

    // Opened: the opener is given the file, already written.
    std::fs::write(&file, "the last one").unwrap();
    let text = show(&sh("test -s \"$1\" && printf '%s' \"$1\" > \"$1.opened\""));
    assert!(
        text.ends_with("wrote .osiris/map.html and opened it in the browser"),
        "{text}"
    );
    let html = std::fs::read_to_string(&file).unwrap();
    assert!(html.contains("id=\"map-data\""), "the last one is replaced");
    assert_eq!(
        std::fs::read_to_string(dir.join(format!("{PAGE_FILE}.opened"))).unwrap(),
        file.to_str().unwrap()
    );

    // The opener reports a failure: said, with the file, and the page
    // is still written.
    std::fs::remove_file(&file).unwrap();
    let text = show(&sh("exit 3"));
    assert!(
        text.contains(
            "wrote .osiris/map.html; it did not open in the browser: `sh` reported a failure"
        ),
        "{text}"
    );
    assert!(
        text.contains("next: open that file in a browser yourself"),
        "{text}"
    );
    assert!(file.exists());

    // There is no opener on this machine: the same.
    let none = Opener {
        program: "ris-no-such-opener".to_owned(),
        args: vec![],
    };
    let text = show(&none);
    assert!(
        text.contains("it did not open in the browser: `ris-no-such-opener` could not be started"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
