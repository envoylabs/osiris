//! Tests live in their own files: no source file in the workspace holds
//! an inline test module, so a source file's diff shows only behaviour.
//! A unit test's module is a file under a `tests` directory beside the
//! source it tests, declared from the source file by a `mod` line.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, leaving out `tests` directories, whose
/// files are the tests themselves.
fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "tests") {
                sources(&path, found);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// The index just past the bracket that closes the one opening `text`,
/// which starts with `open`; strings are stepped over.
fn past_close(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + close.len_utf8());
            }
        }
    }
    None
}

/// Whether a `cfg` predicate holds under test: it names `test` outside
/// any `not(..)` and outside any string, so `all(test, unix)` and
/// `any(test, debug_assertions)` do, and `not(test)` and
/// `feature = "test"` do not.
fn gates_tests(predicate: &str) -> bool {
    // Blank out strings, then every `not(..)` group.
    let mut plain = String::new();
    let mut in_string = false;
    for c in predicate.chars() {
        if c == '"' {
            in_string = !in_string;
            plain.push(' ');
        } else {
            plain.push(if in_string { ' ' } else { c });
        }
    }
    while let Some(at) = plain.find("not(") {
        let end = past_close(&plain[at + 3..], '(', ')').map_or(plain.len(), |n| at + 3 + n);
        plain.replace_range(at..end, " ");
    }
    plain
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "test")
}

/// The lines of `text` that open a module with a body under a `cfg`
/// attribute that holds under test: `#[cfg(test)]`, or one that names
/// `test` among other conditions. The module may follow on the
/// attribute's own line or after other attributes and comments, and
/// may carry a visibility.
fn inline_test_modules(text: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("#[cfg(") {
        let at = from + found;
        from = at + 1;
        let Some(end) = past_close(&text[at + 1..], '[', ']').map(|n| at + 1 + n) else {
            continue;
        };
        // Inside the brackets: `cfg(<predicate>)`.
        if !gates_tests(&text[at + 6..end - 2]) {
            continue;
        }
        // Past any further attributes and comment lines, to the item.
        let mut item = end;
        loop {
            let rest = &text[item..];
            let skipped = rest.len() - rest.trim_start().len();
            let rest = &rest[skipped..];
            if rest.starts_with("#[") {
                match past_close(&rest[1..], '[', ']') {
                    Some(n) => item += skipped + 1 + n,
                    None => break,
                }
            } else if rest.starts_with("//") {
                item += skipped + rest.find('\n').unwrap_or(rest.len());
            } else {
                item += skipped;
                break;
            }
        }
        let mut decl = &text[item..];
        if let Some(rest) = decl.strip_prefix("pub") {
            let rest = rest.trim_start();
            decl = match rest.starts_with('(') {
                true => past_close(rest, '(', ')').map_or(rest, |n| &rest[n..]),
                false => rest,
            }
            .trim_start();
        }
        let Some(body) = decl.strip_prefix("mod ") else {
            continue;
        };
        // A body before any `;` is an inline module; `mod name;` is a
        // declaration of a file.
        let end = body.find(';');
        if body
            .find('{')
            .is_some_and(|open| end.is_none_or(|end| open < end))
        {
            hits.push(text[..item].matches('\n').count() + 1);
        }
    }
    hits
}

#[test]
fn no_source_file_holds_an_inline_test_module() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    assert!(
        files.len() >= 20,
        "only {} source files found; is the workspace there?",
        files.len()
    );
    let mut inline = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        for line in inline_test_modules(&text) {
            let name = file.strip_prefix(&crates).unwrap_or(file);
            inline.push(format!("{}:{line}", name.display()));
        }
    }
    assert!(
        inline.is_empty(),
        "inline test modules, which belong in a file under tests/ beside the source: {inline:?}"
    );
}

/// Every `.rs` file in a `tests` directory under `dir`, at any depth.
fn unit_test_files(dir: &Path, in_tests: bool, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let tests = in_tests || path.file_name().is_some_and(|n| n == "tests");
            unit_test_files(&path, tests, found);
        } else if in_tests && path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn the_map_leaves_out_every_unit_test_file_by_default() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for entry in fs::read_dir(root.join("crates")).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            unit_test_files(&src, false, &mut files);
        }
    }
    assert!(
        files.len() >= 20,
        "only {} unit test files found; is the workspace there?",
        files.len()
    );
    let patterns = ris::config::default_map_tests();
    for file in &files {
        // As git names it: relative to the repository root, `/` between segments.
        let rel = file.strip_prefix(&root).unwrap();
        let parts: Vec<_> = rel.iter().map(|p| p.to_string_lossy()).collect();
        let path = parts.join("/");
        assert!(
            patterns.iter().any(|p| ris::map::glob(p, &path)),
            "{path} is a unit test file the map's default tests pattern misses"
        );
    }
}

#[test]
fn the_scan_finds_an_inline_module_and_passes_a_declared_one() {
    // Assembled from pieces so this file does not itself read as holding one.
    let attr = "#[cfg(test)]";
    let inline = format!("fn a() {{}}\n\n{attr}\nmod tests {{\n    use super::*;\n}}\n");
    assert_eq!(inline_test_modules(&inline), vec![4]);
    let declared = format!("fn a() {{}}\n\n{attr}\n#[path = \"tests/a.rs\"]\nmod tests;\n");
    assert!(inline_test_modules(&declared).is_empty());
    let gated_item = format!("{attr}\nfn only_in_tests() {{}}\n");
    assert!(inline_test_modules(&gated_item).is_empty());

    // Opened on the attribute's own line.
    let one_line = format!("fn a() {{}}\n{attr} mod tests {{\n    use super::*;\n}}\n");
    assert_eq!(inline_test_modules(&one_line), vec![2]);
    let one_line_declared = format!("{attr} mod tests;\nmod other {{}}\n");
    assert!(inline_test_modules(&one_line_declared).is_empty());

    // Under a condition that names test among others, with a
    // visibility, and past a comment and another attribute.
    let cfg = |predicate: &str| format!("#[{}({predicate})]", "cfg");
    let all = format!(
        "{}\nmod tests {{\n}}\n",
        cfg("all(test, feature = \"slow\")")
    );
    assert_eq!(inline_test_modules(&all), vec![2]);
    let any = format!(
        "fn a() {{}}\n{}\n// helpers\n#[allow(dead_code)]\npub(crate) mod support {{\n}}\n",
        cfg("any(test, debug_assertions)")
    );
    assert_eq!(inline_test_modules(&any), vec![5]);
    let all_declared = format!(
        "{}\n#[path = \"tests/a.rs\"]\nmod tests;\n",
        cfg("all(test, unix)")
    );
    assert!(inline_test_modules(&all_declared).is_empty());

    // A condition that does not hold under test is not a test module:
    // the opposite of test, and a feature that is only named so.
    let not = format!("{}\nmod real {{\n}}\n", cfg("not(test)"));
    assert!(inline_test_modules(&not).is_empty());
    let nested_not = format!(
        "{}\nmod real {{\n}}\n",
        cfg("all(unix, not(any(test, windows)))")
    );
    assert!(inline_test_modules(&nested_not).is_empty());
    let feature = format!("{}\nmod extra {{\n}}\n", cfg("feature = \"test\""));
    assert!(inline_test_modules(&feature).is_empty());
    let other = format!("{}\nmod unix_only {{\n}}\n", cfg("unix"));
    assert!(inline_test_modules(&other).is_empty());

    // Two in one file are both found.
    let two = format!("{inline}{one_line}");
    assert_eq!(inline_test_modules(&two), vec![4, 8]);
}
