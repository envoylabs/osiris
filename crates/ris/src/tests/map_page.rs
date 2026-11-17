//! The map page's script, run: the page `page` writes is taken apart
//! into its markup, its document and its script, and the script runs in
//! a JavaScript engine against a stand-in for the browser
//! (`map_page.js`), so an error in it at run time fails a test. d3 is a
//! stub there: the graph's drawing is not run, the list, the view
//! switch and the diff pane are.

use super::*;
use boa_engine::{Context, Source};

fn tree(files: &[(&str, &str)]) -> Tree {
    files
        .iter()
        .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
        .collect()
}

/// The stand-in for the browser, and d3's stub.
const STAND_IN: &str = include_str!("map_page.js");

/// The elements of the page's markup that the script can reach: each
/// with an id, and the first of each tag (for `body` and `header`),
/// with its attributes. A small reader of tags, enough for markup that
/// `page` writes; it is not an HTML parser.
fn elements(markup: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut rest = markup;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(rest.len());
        let tag = &rest[..name_len];
        if tag.is_empty() || !tag.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let mut attrs = serde_json::Map::new();
        let mut s = &rest[name_len..];
        loop {
            s = s.trim_start();
            if s.starts_with('>') || s.starts_with("/>") || s.is_empty() {
                break;
            }
            let end = s
                .find(|c: char| c == '=' || c == '>' || c.is_whitespace())
                .unwrap_or(s.len());
            let name = &s[..end];
            s = &s[end..];
            let value = if let Some(after) = s.strip_prefix("=\"") {
                let close = after.find('"').expect("a closed attribute value");
                s = &after[close + 1..];
                &after[..close]
            } else {
                ""
            };
            attrs.insert(name.to_owned(), value.into());
        }
        rest = s;
        out.push(serde_json::json!({ "tag": tag.to_ascii_lowercase(), "attrs": attrs }));
    }
    out.retain(|e| {
        e["attrs"].get("id").is_some() || matches!(e["tag"].as_str(), Some("body" | "header"))
    });
    out
}

/// The page's parts: its markup up to the document's element, which
/// it includes, the document as the
/// page embeds it, and the script that draws it.
fn parts(html: &str) -> (&str, &str, &str) {
    let open = "<script id=\"map-data\" type=\"application/json\">";
    let data_at = html.find(open).expect("the data element");
    let data = &html[data_at + open.len()..];
    let data = &data[..data.find("</script>").unwrap()];
    let script_at = html.rfind("<script>").expect("the page's script") + "<script>".len();
    let script = &html[script_at..];
    let script = &script[..script.find("</script>").unwrap()];
    (&html[..data_at + open.len()], data, script)
}

/// A page whose script has run in the stand-in, or the error it threw.
struct Page(Context);

impl Page {
    fn run(html: &str, script: &str) -> Result<Page, String> {
        let (markup, data, _) = parts(html);
        let mut page = Page(Context::default());
        page.eval(STAND_IN)?;
        let build = format!(
            "__build({}, {});",
            serde_json::Value::Array(elements(markup)),
            serde_json::Value::String(data.to_owned())
        );
        page.eval(&build)?;
        page.eval(script)?;
        Ok(page)
    }

    fn eval(&mut self, code: &str) -> Result<String, String> {
        let value = self
            .0
            .eval(Source::from_bytes(code))
            .map_err(|e| e.to_string())?;
        if value.is_undefined() {
            return Ok(String::new());
        }
        value
            .to_string(&mut self.0)
            .map(|s| s.to_std_string_escaped())
            .map_err(|e| e.to_string())
    }

    fn state(&mut self) -> serde_json::Value {
        serde_json::from_str(&self.eval("__state()").expect("the page reads back")).unwrap()
    }

    fn act(&mut self, code: &str) {
        if let Err(e) = self.eval(code) {
            panic!("the page's script threw on `{code}`: {e}");
        }
    }
}

/// A phase's map: a judged-high file that sorts last by name, a file
/// that touches the network, a large and a small change, an unchanged
/// file, and the small change's diff. The file that touches the network
/// does so on a line the phase added inside a function, so the document
/// holds a marked line in a function and that function's calls.
fn document_with_changes() -> serde_json::Value {
    serde_json::to_value(map_with_changes(true)).unwrap()
}

/// The same phase read at file level only: no marked line, no changed
/// function and no call graph, as for a language not read inside.
fn document_at_file_level() -> serde_json::Value {
    serde_json::to_value(map_with_changes(false)).unwrap()
}

fn map_with_changes(inside: bool) -> crate::schema::Map {
    let tree = tree(&[
        ("src/a.rs", "pub fn a() {}\n"),
        ("src/big.rs", "pub fn big() {}\n"),
        (
            "src/net.rs",
            "pub fn send() {\n    std::net::TcpStream::connect(\"x\").unwrap();\n}\n",
        ),
        ("src/same.rs", "pub fn same() {}\n"),
        ("src/z.rs", "pub fn z() {}\n"),
    ]);
    let paths: Vec<String> = tree.keys().cloned().collect();
    let diff: BTreeMap<String, (&'static str, u64, u64)> = [
        ("src/a.rs".to_owned(), ("modified", 2, 1)),
        ("src/big.rs".to_owned(), ("added", 300, 0)),
        ("src/net.rs".to_owned(), ("modified", 3, 0)),
        ("src/z.rs".to_owned(), ("modified", 1, 1)),
    ]
    .into();
    let judged: BTreeMap<String, Judged> = [(
        "src/z.rs".to_owned(),
        Judged {
            level: RiskLevel::High,
            why: "changes the format".to_owned(),
            seat: "wren".to_owned(),
        },
    )]
    .into();
    let range = Range {
        phase: None,
        base: Some("base".to_owned()),
        tip: "tip".to_owned(),
        tip_ref: "HEAD".to_owned(),
    };
    let config = crate::config::MapConfig::default();
    let mut map = document(&range, &paths, &tree, &diff, &config, &judged);
    map.diffs.insert(
        "src/a.rs".to_owned(),
        "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n\
         @@ -1,2 +1,3 @@\n keep\n-old\n+new\n+more\n"
            .to_owned(),
    );
    if inside {
        map.diffs.insert(
            "src/net.rs".to_owned(),
            "diff --git a/src/net.rs b/src/net.rs\n--- a/src/net.rs\n+++ b/src/net.rs\n\
             @@ -0,0 +1,3 @@\n+pub fn send() {\n\
             +    std::net::TcpStream::connect(\"x\").unwrap();\n+}\n"
                .to_owned(),
        );
        annotate(&mut map, &tree, &config);
    }
    map
}

fn the_page() -> (serde_json::Value, String) {
    let doc = document_with_changes();
    let html = page(&doc);
    (doc, html)
}

/// The marked lines of `doc` that sit in a function, as (path, line).
fn marks_in_functions(doc: &serde_json::Value) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for n in doc["nodes"].as_array().unwrap() {
        for m in n["marks"].as_array().into_iter().flatten() {
            if m["function"].is_string() {
                out.push((
                    n["path"].as_str().unwrap().to_owned(),
                    m["line"].as_u64().unwrap(),
                ));
            }
        }
    }
    out
}

/// The page's script as it stood when it failed on a real phase: the
/// function that finds a marked line's function assigned after the list
/// that calls it is built.
fn with_the_lookup_after_the_list(script: &str) -> String {
    let open = "  var functionAt = function (path, name, line) {";
    let at = script.find(open).expect("the lookup's definition");
    let close = "\n  };\n";
    let end = at + script[at..].find(close).expect("the lookup's end") + close.len();
    let lookup = &script[at..end];
    let rest = format!("{}{}", &script[..at], &script[end..]);
    let after = "  var callsView = document.getElementById(\"calls\");\n";
    assert!(rest.contains(after), "the line the list ends before");
    rest.replacen(after, &format!("{after}{lookup}"), 1)
}

fn ranked(doc: &serde_json::Value) -> Vec<String> {
    doc["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect()
}

fn row_paths(state: &serde_json::Value) -> Vec<String> {
    state["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_list_s_rows_follow_the_document_s_ranked_order() {
    let (doc, html) = the_page();
    let ranked = ranked(&doc);
    let mut by_name = ranked.clone();
    by_name.sort();
    assert_ne!(
        ranked, by_name,
        "the fixture's order is not the paths' order"
    );
    assert_eq!(ranked[0], "src/z.rs", "the judged-high file leads");
    assert!(!ranked.contains(&"src/same.rs".to_owned()));
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let state = page.state();
    assert_eq!(
        row_paths(&state),
        ranked,
        "one row per ranked file, in order"
    );
}

#[test]
fn the_page_opens_on_the_list_and_the_view_switch_shows_each_view() {
    let html = page(&document_at_file_level());
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let views = |s: &serde_json::Value| {
        let p = &s["pressed"];
        (
            s["body"].as_str().unwrap().to_owned(),
            p["list"].as_str().unwrap().to_owned(),
            p["graph"].as_str().unwrap().to_owned(),
            p["calls"].as_str().unwrap().to_owned(),
        )
    };
    let shown = |body: &str| {
        (
            body.split(' ').any(|c| c == "listing"),
            body.split(' ').any(|c| c == "calling"),
        )
    };

    let s = page.state();
    let (body, list, graph, calls) = views(&s);
    assert_eq!(shown(&body), (true, false), "{body}");
    assert_eq!(
        (list.as_str(), graph.as_str(), calls.as_str()),
        ("true", "false", "false")
    );

    page.act("__click('view-graph')");
    let (body, list, graph, calls) = views(&page.state());
    assert_eq!(shown(&body), (false, false), "{body}");
    assert_eq!(
        (list.as_str(), graph.as_str(), calls.as_str()),
        ("false", "true", "false")
    );

    // This document has no call graph: the calls view shows and says
    // what to do instead.
    page.act("__click('view-calls')");
    let s = page.state();
    let (body, list, graph, calls) = views(&s);
    assert_eq!(shown(&body), (false, true), "{body}");
    assert_eq!(
        (list.as_str(), graph.as_str(), calls.as_str()),
        ("false", "false", "true")
    );
    assert!(
        s["calls_note"]
            .as_str()
            .unwrap()
            .starts_with("No changed function"),
        "{s}"
    );

    page.act("__click('view-list')");
    let (body, list, graph, calls) = views(&page.state());
    assert_eq!(shown(&body), (true, false), "{body}");
    assert_eq!(
        (list.as_str(), graph.as_str(), calls.as_str()),
        ("true", "false", "false")
    );
}

#[test]
fn a_row_opens_its_file_s_diff_in_the_pane_and_escape_closes_it() {
    let (doc, html) = the_page();
    let ranked = ranked(&doc);
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let paned =
        |s: &serde_json::Value| s["body"].as_str().unwrap().split(' ').any(|c| c == "paned");
    assert!(!paned(&page.state()));

    let a = ranked.iter().position(|p| p == "src/a.rs").unwrap();
    page.act(&format!("__clickRow({a})"));
    let s = page.state();
    assert!(paned(&s), "{s}");
    assert_eq!(s["pane_path"], "src/a.rs");
    // The file header is left out; the hunk, then its lines.
    assert_eq!(
        s["pane_rows"],
        serde_json::json!(["hunk", "ctx", "del", "add", "add"])
    );
    let picked: Vec<bool> = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["picked"].as_bool().unwrap())
        .collect();
    assert_eq!(
        picked,
        (0..ranked.len()).map(|i| i == a).collect::<Vec<_>>(),
        "only the clicked row is picked"
    );

    // Another row swaps the file; one with no diff in the document says
    // so.
    page.act("__clickRow(0)");
    let s = page.state();
    assert_eq!(s["pane_path"], ranked[0].as_str());
    assert_eq!(s["pane_rows"], serde_json::json!([]));
    assert_eq!(s["pane_text"], "The document holds no diff for this file.");

    page.act("__key('Escape')");
    let s = page.state();
    assert!(!paned(&s), "{s}");
    assert!(
        s["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["picked"] == false)
    );
}

#[test]
fn a_marked_line_in_a_function_opens_the_calls_view_on_that_function() {
    let (doc, html) = the_page();
    assert_eq!(
        marks_in_functions(&doc),
        [("src/net.rs".to_owned(), 2)],
        "the fixture holds a marked line inside a function"
    );
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let s = page.state();
    assert_eq!(row_paths(&s), ranked(&doc), "every row is drawn");
    assert_eq!(
        s["marks"],
        serde_json::json!([{ "text": "line 2  network in send (low)", "opens": true }]),
        "{s}"
    );
    assert_eq!(s["list_top"], "52px", "the list is placed below the header");

    page.act("__clickMark(0)");
    let s = page.state();
    assert!(
        s["body"]
            .as_str()
            .unwrap()
            .split(' ')
            .any(|c| c == "calling"),
        "{s}"
    );
    assert_eq!(
        s["calls_note"],
        "send in src/net.rs: click a box beside it to move there."
    );
    assert_eq!(s["pane_path"], "", "opening a function picks no file");
}

#[test]
fn the_one_step_marks_are_folded_under_a_count_below_the_direct_marks() {
    // The file that touches the network, with what `annotate` would say
    // of a larger repository: `send` one step from an effect twice, once
    // through a wide function; a second function through the same wide
    // one; and a wide function of the file's own.
    let mut doc = document_with_changes();
    let step = |direction: &str, function: &str, wide: bool| {
        serde_json::json!({
            "direction": direction, "path": "src/main.rs", "function": function,
            "effects": ["filesystem"], "wide": wide,
        })
    };
    let net = doc["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["path"] == "src/net.rs")
        .unwrap();
    let mut send = net["functions"][0].clone();
    assert_eq!(send["name"], "send");
    send["steps"] =
        serde_json::json!([step("feeds", "run", true), step("triggers", "save", false)]);
    let mut other = send.clone();
    other["name"] = "other".into();
    other["steps"] = serde_json::json!([step("feeds", "run", true)]);
    let mut dispatch = send.clone();
    dispatch["name"] = "dispatch".into();
    dispatch["wide"] = true.into();
    dispatch["steps"] = serde_json::json!([
        step("triggers", "save", false),
        step("triggers", "load", false)
    ]);
    net["functions"] = serde_json::json!([send, other, dispatch]);

    let html = page(&doc);
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let s = page.state();
    // The direct mark stands outside the fold, and first.
    assert_eq!(s["marks"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(
        s["folds"],
        serde_json::json!([{ "label": "▸ 3 one step from an effect", "hidden": true }]),
        "the fold is closed, and a wide function counts once"
    );
    let most = ", which most of the repository calls or is called by";
    assert_eq!(
        s["steps"],
        serde_json::json!([
            { "text": "send triggers filesystem via save  src/main.rs", "opens": true },
            // Neither `dispatch` nor `run` is in this document's call
            // graph, so neither line opens a function.
            { "text": format!("dispatch{most}, is one step from 2 functions with an effect"),
              "opens": false },
            { "text": format!("2 functions feed filesystem via run  src/main.rs{most}"),
              "opens": false },
        ])
    );

    page.act("__clickUnfold(0)");
    let s = page.state();
    assert_eq!(
        s["folds"],
        serde_json::json!([{ "label": "▾ 3 one step from an effect", "hidden": false }])
    );
    assert_eq!(s["pane_path"], "", "unfolding picks no file");
    page.act("__clickStep(0)");
    assert_eq!(
        page.state()["calls_note"],
        "send in src/net.rs: click a box beside it to move there."
    );
}

#[test]
fn the_calls_button_opens_on_the_first_changed_function() {
    let (_, html) = the_page();
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    page.act("__click('view-calls')");
    let s = page.state();
    assert_eq!(s["pressed"]["calls"], "true", "{s}");
    assert_eq!(
        s["calls_note"],
        "send in src/net.rs: click a box beside it to move there."
    );
}

#[test]
fn the_page_draws_this_repository_s_last_closed_phase() {
    // Real data: the last phase closed on this repository's history.
    let Some(map) = super::history::closed_phase("chore: close P-") else {
        return;
    };
    let doc = serde_json::to_value(&map).unwrap();
    let in_functions = marks_in_functions(&doc);
    assert!(
        !in_functions.is_empty(),
        "the phase holds a marked line inside a function"
    );

    let html = page(&doc);
    let (_, _, script) = parts(&html);
    let mut page = Page::run(&html, script).unwrap_or_else(|e| panic!("the script threw: {e}"));
    let s = page.state();
    let ranked = ranked(&doc);
    assert!(!ranked.is_empty());
    assert_eq!(row_paths(&s), ranked, "one row per ranked file, in order");
    assert_eq!(s["list_top"], "52px", "the list is placed below the header");
    let opening = s["marks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["opens"] == true)
        .count();
    assert!(
        opening >= in_functions.len(),
        "each marked line in a function the call graph holds opens it: {opening} of {}",
        in_functions.len()
    );

    // The view buttons have their listeners, and each view shows.
    let shows = |s: &serde_json::Value, class: &str| {
        s["body"].as_str().unwrap().split(' ').any(|c| c == class)
    };
    page.act("__click('view-graph')");
    let s = page.state();
    assert_eq!(s["pressed"]["graph"], "true", "{}", s["pressed"]);
    assert!(!shows(&s, "listing") && !shows(&s, "calling"));
    page.act("__click('view-calls')");
    let s = page.state();
    assert_eq!(s["pressed"]["calls"], "true", "{}", s["pressed"]);
    assert!(shows(&s, "calling"));
    assert!(
        s["calls_note"]
            .as_str()
            .unwrap()
            .ends_with("to move there."),
        "the calls view opens on a function: {}",
        s["calls_note"]
    );
    page.act("__click('view-list')");
    assert!(shows(&page.state(), "listing"));

    // A marked line opens its function, and a row its diff.
    page.act("__clickMark(0)");
    assert!(shows(&page.state(), "calling"));
    page.act("__clickRow(0)");
    let s = page.state();
    assert_eq!(s["pane_path"], ranked[0].as_str());
    assert!(!s["pane_rows"].as_array().unwrap().is_empty());
}

#[test]
fn the_lookup_assigned_after_the_list_fails_on_a_marked_line_in_a_function() {
    // The falsifier of the two tests above: the script as it stood, over
    // the same document, stops while the list is built; over a document
    // with no marked line in a function it runs, which is how the fault
    // went unseen.
    let (_, html) = the_page();
    let (_, _, script) = parts(&html);
    let broken = with_the_lookup_after_the_list(script);
    assert_ne!(broken, script);
    let err = Page::run(&html, &broken)
        .err()
        .expect("the list's first marked line throws");
    assert!(err.contains("not a callable function"), "{err}");

    let html = page(&document_at_file_level());
    assert!(Page::run(&html, &broken).is_ok());
}

#[test]
fn an_error_in_the_script_at_run_time_fails_here() {
    // The falsifier of the tests above: a script that loads but throws
    // when a row is clicked, and one that throws as it loads.
    let (_, html) = the_page();
    let (_, _, script) = parts(&html);
    let click = "tr.addEventListener(\"click\", function () { pick(n); });";
    assert!(script.contains(click));
    let broken = script.replacen(click, &click.replace("pick(n)", "pikc(n)"), 1);
    let mut page = Page::run(&html, &broken).unwrap_or_else(|e| panic!("it loads: {e}"));
    let err = page.eval("__clickRow(0)").expect_err("the click throws");
    assert!(err.contains("pikc"), "{err}");

    let broken = script.replacen(
        "var listRows = document.getElementById(\"list-rows\");",
        "var listRows = document.getElementById(\"list-rowz\");",
        1,
    );
    assert_ne!(broken, script);
    assert!(
        Page::run(&html, &broken).is_err(),
        "an element the markup lacks"
    );
}
