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

/// The lines of `text` that open a module with a body right after a
/// `#[cfg(test)]` attribute, other attributes between them allowed.
fn inline_test_modules(text: &str) -> Vec<usize> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if *line != "#[cfg(test)]" {
            continue;
        }
        let next = lines[i + 1..]
            .iter()
            .position(|l| !l.starts_with("#["))
            .map(|n| (i + 1 + n, lines[i + 1 + n]));
        if let Some((at, decl)) = next
            && (decl.starts_with("mod ") || decl.starts_with("pub mod "))
            && !decl.ends_with(';')
        {
            hits.push(at + 1);
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
}
