//! How files connect in TypeScript, Go and Clojure, for `ris map`: each
//! a [`Language`] beside Rust's, so a file of these languages has edges
//! to the files it imports and counts the files that import it.
//!
//! Each reads a file's imports from its syntax tree
//! ([`crate::syntax::parse`]) and finds the file they name in the tree:
//!
//! - TypeScript: a relative module specifier (`./store`, `../wire.js`)
//!   is a file beside the importer, with or without its extension, or a
//!   directory's `index` file. A package name is no file of the tree.
//! - Go: an import path under a module that a `go.mod` in the tree
//!   declares is that module's directory, and the edges go to every Go
//!   file in it.
//! - Clojure: a required namespace is every file whose `ns` form
//!   declares it.
//!
//! A file touches the outside when a name it uses, or a module it
//! imports, is one of the language's shipped effect patterns
//! ([`crate::syntax::default_effects`]), as Rust's scan does with its
//! own.

use std::collections::{BTreeMap, BTreeSet};

use crate::map::{Language, Scan, Tree, language_of};
use crate::syntax::{self, Parsed};

/// The parse of every file of `language` in `tree`.
fn parsed<'t>(language: &str, tree: &'t Tree) -> Vec<(&'t String, Parsed)> {
    tree.iter()
        .filter(|(path, _)| language_of(path) == Some(language))
        .filter_map(|(path, text)| Some((path, syntax::parse(language, path, text)?)))
        .collect()
}

/// The kinds of effect `parsed` shows: a name it uses or a module it
/// imports that a pattern covers.
fn external(language: &str, parsed: &Parsed) -> BTreeSet<&'static str> {
    let patterns = syntax::default_effects(language);
    let names = parsed
        .references
        .iter()
        .map(|r| r.name.as_str())
        .chain(parsed.imports.iter().map(String::as_str));
    let mut out = BTreeSet::new();
    for name in names {
        for (pattern, kind) in &patterns {
            if syntax::matches(pattern, name) {
                out.insert(*kind);
            }
        }
    }
    out
}

/// The directory a path is in; empty at the root.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `rel` taken from `dir`, with `.` and `..` worked out; `None` when it
/// climbs above the root.
fn resolve(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// TypeScript: relative imports, resolved as a bundler or `tsc` does.
pub struct TypeScript;

/// What a TypeScript module may be written as, after its stem.
const TS_EXTENSIONS: &[&str] = &[".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs"];

impl TypeScript {
    /// The file a relative specifier from `file` names, if it is in
    /// `tree`.
    fn target(file: &str, spec: &str, tree: &Tree) -> Option<String> {
        if !spec.starts_with("./") && !spec.starts_with("../") {
            return None;
        }
        let base = resolve(parent(file), spec)?;
        // Written with the extension of what it compiles to (`./wire.js`
        // for `wire.ts`), or of itself.
        let stem = [".js", ".jsx", ".mjs", ".cjs"]
            .iter()
            .find_map(|e| base.strip_suffix(e));
        let mut candidates = vec![base.clone()];
        if let Some(stem) = stem {
            candidates.extend([".ts", ".tsx", ".mts", ".cts"].map(|e| format!("{stem}{e}")));
        }
        candidates.extend(TS_EXTENSIONS.iter().map(|e| format!("{base}{e}")));
        candidates.extend(TS_EXTENSIONS.iter().map(|e| format!("{base}/index{e}")));
        candidates.into_iter().find(|c| tree.contains_key(c))
    }
}

impl Language for TypeScript {
    fn name(&self) -> &'static str {
        "typescript"
    }

    fn scan(&self, tree: &Tree) -> BTreeMap<String, Scan> {
        let mut out = BTreeMap::new();
        for (file, parsed) in parsed(self.name(), tree) {
            let mut scan = Scan {
                external: external(self.name(), &parsed),
                ..Scan::default()
            };
            for spec in &parsed.imports {
                if let Some(to) = Self::target(file, spec, tree)
                    && to != *file
                {
                    scan.edges.insert((to, "import"));
                }
            }
            out.insert(file.clone(), scan);
        }
        out
    }
}

/// Go: import paths under the tree's own modules.
pub struct Go;

impl Go {
    /// Each `go.mod` in the tree: its directory, and the module path it
    /// declares.
    fn modules(tree: &Tree) -> Vec<(String, String)> {
        tree.iter()
            .filter(|(path, _)| path.rsplit('/').next() == Some("go.mod"))
            .filter_map(|(path, text)| {
                let module = text.lines().find_map(|l| {
                    let rest = l.trim().strip_prefix("module")?;
                    rest.starts_with(char::is_whitespace)
                        .then(|| rest.trim().trim_matches('"').to_owned())
                })?;
                Some((parent(path).to_owned(), module))
            })
            .collect()
    }

    /// The directory in the tree an import path names, if a module of
    /// the tree holds it.
    fn directory(path: &str, modules: &[(String, String)]) -> Option<String> {
        modules.iter().find_map(|(dir, module)| {
            let rest = if path == module {
                ""
            } else {
                path.strip_prefix(module.as_str())?.strip_prefix('/')?
            };
            resolve(dir, rest)
        })
    }
}

impl Language for Go {
    fn name(&self) -> &'static str {
        "go"
    }

    fn manifests(&self) -> &'static [&'static str] {
        &["go.mod"]
    }

    fn scan(&self, tree: &Tree) -> BTreeMap<String, Scan> {
        let modules = Self::modules(tree);
        // The Go files of each directory, a package's files.
        let mut packages: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
        for path in tree.keys().filter(|p| language_of(p) == Some("go")) {
            packages.entry(parent(path)).or_default().push(path);
        }
        let mut out = BTreeMap::new();
        for (file, parsed) in parsed(self.name(), tree) {
            let mut scan = Scan {
                external: external(self.name(), &parsed),
                ..Scan::default()
            };
            for import in &parsed.imports {
                let Some(dir) = Self::directory(import, &modules) else {
                    continue;
                };
                for to in packages.get(dir.as_str()).into_iter().flatten() {
                    if *to != file {
                        scan.edges.insert(((*to).clone(), "import"));
                    }
                }
            }
            out.insert(file.clone(), scan);
        }
        out
    }
}

/// Clojure: required namespaces, by the `ns` forms that declare them.
pub struct Clojure;

impl Language for Clojure {
    fn name(&self) -> &'static str {
        "clojure"
    }

    fn scan(&self, tree: &Tree) -> BTreeMap<String, Scan> {
        let files = parsed(self.name(), tree);
        let mut declared: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
        for (file, parsed) in &files {
            if let Some(ns) = &parsed.namespace {
                declared.entry(ns.as_str()).or_default().push(file);
            }
        }
        let mut out = BTreeMap::new();
        for (file, parsed) in &files {
            let mut scan = Scan {
                external: external(self.name(), parsed),
                ..Scan::default()
            };
            for ns in &parsed.imports {
                for to in declared.get(ns.as_str()).into_iter().flatten() {
                    if *to != *file {
                        scan.edges.insert(((*to).clone(), "require"));
                    }
                }
            }
            out.insert((*file).clone(), scan);
        }
        out
    }
}

#[cfg(test)]
#[path = "tests/connect.rs"]
mod tests;
