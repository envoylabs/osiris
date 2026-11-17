//! `ris map`: the phase's change set drawn over the source code, for
//! the person to read at review.
//!
//! The graph document is the contract (`docs/json.md`): its nodes are
//! the repository's source files, each with the lines the phase added
//! and removed, and its edges are how the code connects. A page
//! (`ris map --html`) draws that document and holds nothing the
//! document does not, so any other renderer draws the same thing.
//!
//! Everything is read from git objects at the phase's tip, never from a
//! checkout: the phase branch is checked out nowhere, and the person's
//! own checkout stays on the main line.
//!
//! How code connects is found per language, behind [`Language`]. Rust
//! is the first: `mod` lines, `use` lines and qualified paths, read by
//! scanning the text. A file of a language with no scanner is still a
//! node, with its diff and no edges.
//!
//! Inside a changed file of Rust, TypeScript, Go or Clojure, the
//! functions the phase changed and the lines that touch the outside are
//! read from its syntax tree ([`crate::syntax`]) and its diff, by
//! [`annotate`]; a file of any other language stays at file level.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::project::{CliError, Loaded, Project};
use crate::render::Output;
use ris_core::event::Structure;
use ris_core::ids::{Actor, PhaseId};
use ris_core::primitives::{PhaseState, RiskLevel};
use serde::Serialize;

/// File extensions that are source code, and the language each names.
/// A file with none of these is not a node unless a contract pattern
/// names it.
const SOURCE: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascript"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("go", "go"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("swift", "swift"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
    ("cs", "csharp"),
    ("rb", "ruby"),
    ("php", "php"),
    ("scala", "scala"),
    ("sh", "shell"),
    ("sql", "sql"),
    ("proto", "protobuf"),
    ("clj", "clojure"),
    ("cljs", "clojure"),
    ("cljc", "clojure"),
];

/// The language a path's extension names, if it is source code. A
/// minified build is a vendored artefact, not source.
pub fn language_of(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.contains(".min.") {
        return None;
    }
    let ext = name.rsplit_once('.')?.1;
    SOURCE.iter().find(|(e, _)| *e == ext).map(|(_, l)| *l)
}

/// The files a scan reads: path to text, as they are at the phase's
/// tip. Holds every source file and every manifest a language asked for.
pub type Tree = BTreeMap<String, String>;

/// What a scan found in one file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// The files this one connects to, each with the kind of line that
    /// says so.
    pub edges: BTreeSet<(String, &'static str)>,
    /// How the file touches the outside: any of `database`,
    /// `filesystem`, `network`, `process`.
    pub external: BTreeSet<&'static str>,
}

/// The seam for a language: given the tree, how its files connect and
/// which touch the outside. Adding a language adds an implementation
/// here and to [`LANGUAGES`]; the document does not change.
pub trait Language {
    /// The name [`language_of`] gives its files.
    fn name(&self) -> &'static str;
    /// File names, beside the source, the scan needs to read.
    fn manifests(&self) -> &'static [&'static str] {
        &[]
    }
    /// Every file of this language in `tree` that has something to say.
    fn scan(&self, tree: &Tree) -> BTreeMap<String, Scan>;
}

/// The languages with a scanner.
pub const LANGUAGES: &[&dyn Language] = &[&Rust];

/// One source file.
#[derive(Clone, Debug, Serialize, PartialEq, schemars::JsonSchema)]
pub struct Node {
    /// Relative to the repository root.
    pub path: String,
    /// The language its extension names; null for a contract file that
    /// is not source code.
    pub language: Option<&'static str>,
    /// `added`, `modified`, `deleted` or `unchanged`, in the phase.
    pub status: &'static str,
    /// Lines the phase added and removed.
    pub added: u64,
    pub removed: u64,
    /// How it touches the outside; empty when it does not.
    pub external: Vec<&'static str>,
    /// Whether a `[map] contracts` pattern names it.
    pub contract: bool,
    /// How much it should draw the eye: grows with the diff's size and
    /// with being external or a contract.
    pub weight: f64,
    /// How many other files on the map have an edge to it.
    pub dependents: usize,
    /// How closely its change should be read; null for a file the phase
    /// did not change.
    pub risk: Option<Risk>,
    /// The functions the phase changed, in the order they start; null
    /// unless the file is changed, still there, has a diff, and is of a
    /// language the map reads inside (`syntax::SUPPORTED`).
    pub functions: Option<Vec<ChangedFunction>>,
    /// The lines that touch the outside, on a line the phase changed or
    /// in a function it changed, by line; null when `functions` is.
    pub marks: Option<Vec<Mark>>,
}

/// One function the phase changed.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ChangedFunction {
    /// As a reader finds it; a method has its type in front.
    pub name: String,
    /// Its first and last line at the tip, counting from 1.
    pub start: u64,
    pub end: u64,
    /// The lines the phase added in it, and removed from it.
    pub added: u64,
    pub removed: u64,
    /// The kinds of effect on its lines, changed or not; sorted.
    pub effects: Vec<&'static str>,
    /// Whether it holds an effect site itself.
    pub direct: bool,
    /// The functions of the repository it calls, as far as names say:
    /// a call that could mean more than one function names each.
    pub calls: Vec<Callee>,
    /// One step from an effect: each direct function it calls (it
    /// triggers the effect) and each that calls it (it feeds the
    /// effect).
    pub steps: Vec<Step>,
    /// Its level, from the same signals as a file's (its effects and
    /// the size of its change), and one more for being one step from an
    /// effect.
    pub risk: Computed,
}

/// A function of the repository, by file and name.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
pub struct Callee {
    pub path: String,
    pub function: String,
}

/// A changed function one step from an effect.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
pub struct Step {
    /// `triggers`: it calls the direct function; `feeds`: the direct
    /// function calls it.
    pub direction: &'static str,
    /// The direct function, by file and name.
    pub path: String,
    pub function: String,
    /// The kinds of effect that function holds; sorted.
    pub effects: Vec<&'static str>,
}

/// A line that touches the outside.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct Mark {
    /// At the tip, counting from 1.
    pub line: u64,
    /// `database`, `filesystem`, `network` or `process`.
    pub kind: &'static str,
    /// The innermost function it sits in; null outside any.
    pub function: Option<String>,
    /// That function's level, which is the line's; null outside any
    /// function, where the file's level stands.
    pub risk: Option<RiskLevel>,
}

/// What a file's unified diff changed, in the tip's line numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The lines it added.
    pub added: BTreeSet<u64>,
    /// For each run of removed lines, the tip's line it sat before, and
    /// how many it removed there.
    pub removed: BTreeMap<u64, u64>,
}

/// The lines a unified diff (as `git diff` writes it) added and where
/// it removed lines, read from its hunks.
pub fn changes(diff: &str) -> Changes {
    let mut out = Changes::default();
    let mut now: Option<u64> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            // `@@ -old[,n] +new[,n] @@`
            now = rest
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|n| n.split(',').next())
                .and_then(|n| n.parse().ok());
            continue;
        }
        let Some(at) = now.as_mut() else {
            continue;
        };
        match line.chars().next() {
            Some('+') => {
                out.added.insert(*at);
                *at += 1;
            }
            Some('-') => *out.removed.entry(*at).or_default() += 1,
            Some('\\') => {}
            _ => *at += 1,
        }
    }
    out
}

/// For a function of the file: the functions it calls, and its steps
/// to an effect ([`Graph::reach`]).
pub type Reach<'a> = dyn Fn(&crate::syntax::Function) -> (Vec<Callee>, Vec<Step>) + 'a;

/// The functions `parsed` holds that `changes` touched, and the effect
/// sites on a changed line or in a changed function, by the `patterns`
/// of the file's language.
pub fn inside(
    parsed: &crate::syntax::Parsed,
    changes: &Changes,
    patterns: &[(String, &'static str)],
    reach: &Reach<'_>,
) -> (Vec<ChangedFunction>, Vec<Mark>) {
    let mut sites: BTreeSet<(u64, &'static str)> = BTreeSet::new();
    for r in &parsed.references {
        for (pattern, kind) in patterns {
            if crate::syntax::matches(pattern, &r.name) {
                sites.insert((r.line, kind));
            }
        }
    }
    let within = |f: &crate::syntax::Function, line: u64| f.start <= line && line <= f.end;
    let mut functions = Vec::new();
    for f in &parsed.functions {
        let added = changes.added.iter().filter(|l| within(f, **l)).count() as u64;
        let removed: u64 = changes
            .removed
            .iter()
            .filter(|(l, _)| within(f, **l))
            .map(|(_, n)| n)
            .sum();
        if added + removed == 0 {
            continue;
        }
        let effects: Vec<&'static str> = sites
            .iter()
            .filter(|(l, _)| within(f, *l))
            .map(|(_, k)| *k)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (calls, steps) = reach(f);
        functions.push(ChangedFunction {
            name: f.name.clone(),
            start: f.start,
            end: f.end,
            added,
            removed,
            direct: !effects.is_empty(),
            risk: function_risk(&effects, added + removed, &steps),
            effects,
            calls,
            steps,
        });
    }
    let marks = sites
        .iter()
        .filter_map(|(line, kind)| {
            // The innermost function holding it: the latest to start.
            let holder = parsed
                .functions
                .iter()
                .filter(|f| within(f, *line))
                .max_by_key(|f| (f.start, std::cmp::Reverse(f.end)));
            let changed = holder.and_then(|h| {
                functions.iter().find(|c| {
                    (c.name.as_str(), c.start, c.end) == (h.name.as_str(), h.start, h.end)
                })
            });
            (changes.added.contains(line) || changed.is_some()).then(|| Mark {
                line: *line,
                kind,
                function: holder.map(|h| h.name.clone()),
                risk: changed.map(|c| c.risk.level),
            })
        })
        .collect();
    (functions, marks)
}

/// Fill in each changed file's `functions` and `marks`, from its text
/// at the tip in `tree` and its diff in the document. Every file on the
/// map of a language read inside is parsed, so that calls between
/// functions are known across the repository.
pub fn annotate(map: &mut crate::schema::Map, tree: &Tree, config: &crate::config::MapConfig) {
    let mut files: BTreeMap<String, (&'static str, crate::syntax::Parsed)> = BTreeMap::new();
    for node in map.nodes.iter().filter(|n| n.status != "deleted") {
        if let (Some(language), Some(text)) = (node.language, tree.get(&node.path))
            && let Some(parsed) = crate::syntax::parse(language, &node.path, text)
        {
            files.insert(node.path.clone(), (language, parsed));
        }
    }
    let graph = Graph::new(&files, &Locals::of(tree, &files), config);
    let mut changed = Vec::new();
    let mut added = BTreeMap::new();
    for node in &mut map.nodes {
        if matches!(node.status, "unchanged" | "deleted") {
            continue;
        }
        let (Some((language, parsed)), Some(diff)) =
            (files.get(&node.path), map.diffs.get(&node.path))
        else {
            continue;
        };
        let reach = |f: &crate::syntax::Function| graph.reach(&node.path, f);
        let lines = changes(diff);
        let (functions, marks) = inside(parsed, &lines, &config.effects_for(language), &reach);
        for c in &functions {
            if let Some(f) = parsed
                .functions
                .iter()
                .find(|f| (f.name.as_str(), f.start, f.end) == (c.name.as_str(), c.start, c.end))
            {
                changed.push((node.path.clone(), f.clone()));
            }
        }
        added.insert(node.path.clone(), lines.added);
        node.functions = Some(functions);
        node.marks = Some(marks);
    }
    map.call_graph = graph.around(&changed, tree, &added);
}

/// What a repository holds that says whether a call's module is its
/// own: its Rust packages, the directories of its Go files, and the
/// namespaces its Clojure files declare. A TypeScript module is the
/// repository's own when imported by a relative path.
#[derive(Debug, Default)]
pub struct Locals {
    pub rust_packages: BTreeSet<String>,
    pub go_dirs: BTreeSet<String>,
    pub clojure_namespaces: BTreeSet<String>,
}

impl Locals {
    pub fn of(
        tree: &Tree,
        files: &BTreeMap<String, (&'static str, crate::syntax::Parsed)>,
    ) -> Self {
        let mut out = Locals {
            // A package's name as code names it, with `_` for `-`.
            rust_packages: Rust::packages(tree)
                .into_keys()
                .map(|n| n.replace('-', "_"))
                .collect(),
            ..Locals::default()
        };
        for (path, (language, parsed)) in files {
            match *language {
                "go" => {
                    out.go_dirs.insert(parent(path).to_owned());
                }
                "clojure" => out.clojure_namespaces.extend(parsed.namespace.clone()),
                _ => {}
            }
        }
        out
    }

    /// Whether a call from `language` into `module` stays in the
    /// repository. No module is the file's own name, or a method on a
    /// value, which may be any of the repository's.
    pub fn is_local(&self, language: &str, module: Option<&str>) -> bool {
        let Some(module) = module else {
            return true;
        };
        match language {
            "rust" => {
                let root = module.split("::").next().unwrap_or(module);
                matches!(root, "crate" | "self" | "super" | "Self")
                    || self.rust_packages.contains(root)
            }
            "typescript" => {
                module.starts_with('.')
                    || module.starts_with('/')
                    || module.starts_with("@/")
                    || module.starts_with("~/")
            }
            "go" => self
                .go_dirs
                .iter()
                .any(|d| !d.is_empty() && (module == d || module.ends_with(&format!("/{d}")))),
            "clojure" => self.clojure_namespaces.contains(module),
            _ => false,
        }
    }
}

/// An effect site in a function: its line and kind.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
pub struct Site {
    pub line: u64,
    pub kind: &'static str,
}

/// The calls around the phase's changed functions, for a view of one
/// function: every changed function and every function one call from
/// one, in full; the functions one call further, by name only.
#[derive(Clone, Debug, Default, Serialize, schemars::JsonSchema)]
pub struct CallGraph {
    /// By path, then first line.
    pub functions: Vec<CallNode>,
}

/// One function of the call graph.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CallNode {
    /// `path:start`, which `calls` and `callers` name it by.
    pub id: String,
    pub path: String,
    pub name: String,
    pub start: u64,
    pub end: u64,
    /// Whether it holds an effect site itself, and their kinds, sorted.
    pub direct: bool,
    pub effects: Vec<&'static str>,
    /// Its effect sites; empty for a function given by name only.
    pub sites: Vec<Site>,
    /// The lines of it the phase added.
    pub changed: Vec<u64>,
    /// Its text at the tip, one entry a line from `start`; null for a
    /// function given by name only.
    pub lines: Option<Vec<String>>,
    /// The functions it calls and those that call it, by id; empty for
    /// a function given by name only.
    pub calls: Vec<String>,
    pub callers: Vec<String>,
}

/// One function of the repository, for the call graph.
struct Known {
    path: String,
    language: &'static str,
    function: crate::syntax::Function,
    effects: Vec<&'static str>,
    /// Its effect sites, by line.
    sites: Vec<Site>,
}

/// Which function of the repository calls which, as far as names say.
pub struct Graph {
    known: Vec<Known>,
    /// By caller, the functions it calls; by callee, those that call it.
    calls: BTreeMap<usize, BTreeSet<usize>>,
    callers: BTreeMap<usize, BTreeSet<usize>>,
}

impl Graph {
    /// The graph over `files`, each call resolved by name to every
    /// function of the same language it could mean: a name with a type
    /// in front to that method, a bare name to every function or method
    /// so named. A call into a module that is not the repository's own
    /// (`locals`) is left out.
    pub fn new(
        files: &BTreeMap<String, (&'static str, crate::syntax::Parsed)>,
        locals: &Locals,
        config: &crate::config::MapConfig,
    ) -> Self {
        let mut known = Vec::new();
        for (path, (language, parsed)) in files {
            let patterns = config.effects_for(language);
            for f in &parsed.functions {
                let sites: BTreeSet<Site> = parsed
                    .references
                    .iter()
                    .filter(|r| f.start <= r.line && r.line <= f.end)
                    .flat_map(|r| {
                        patterns
                            .iter()
                            .filter(|(p, _)| crate::syntax::matches(p, &r.name))
                            .map(|(_, kind)| Site { line: r.line, kind })
                    })
                    .collect();
                let effects: BTreeSet<&'static str> = sites.iter().map(|s| s.kind).collect();
                known.push(Known {
                    path: path.clone(),
                    language,
                    function: f.clone(),
                    effects: effects.into_iter().collect(),
                    sites: sites.into_iter().collect(),
                });
            }
        }
        // Each function under its own name and under its last segment.
        let mut by_name: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
        for (i, k) in known.iter().enumerate() {
            let sep = crate::syntax::method_separator(k.language);
            let name = k.function.name.as_str();
            by_name.entry((k.language, name)).or_default().push(i);
            if let Some((_, last)) = name.rsplit_once(sep) {
                by_name.entry((k.language, last)).or_default().push(i);
            }
        }
        let mut in_file: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, k) in known.iter().enumerate() {
            in_file.entry(k.path.as_str()).or_default().push(i);
        }
        let mut calls: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let mut callers: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (path, (language, parsed)) in files {
            let here = in_file.get(path.as_str()).map_or(&[][..], Vec::as_slice);
            for call in &parsed.calls {
                if !locals.is_local(language, call.module.as_deref()) {
                    continue;
                }
                // The innermost function the call is in.
                let Some(from) = here
                    .iter()
                    .copied()
                    .filter(|i| {
                        let f = &known[*i].function;
                        f.start <= call.line && call.line <= f.end
                    })
                    .max_by_key(|i| {
                        let f = &known[*i].function;
                        (f.start, std::cmp::Reverse(f.end))
                    })
                else {
                    continue;
                };
                for to in by_name
                    .get(&(*language, call.name.as_str()))
                    .into_iter()
                    .flatten()
                {
                    if *to != from {
                        calls.entry(from).or_default().insert(*to);
                        callers.entry(*to).or_default().insert(from);
                    }
                }
            }
        }
        Graph {
            known,
            calls,
            callers,
        }
    }

    /// Where the function `f` of the file at `path` is in the graph.
    fn index_of(&self, path: &str, f: &crate::syntax::Function) -> Option<usize> {
        self.known
            .iter()
            .position(|k| k.path == path && k.function == *f)
    }

    /// The call graph around `changed` (functions by path), in full one
    /// call out and by name one call further: the text of each from
    /// `tree`, and the lines of it the phase added from `added` (by
    /// path).
    pub fn around(
        &self,
        changed: &[(String, crate::syntax::Function)],
        tree: &Tree,
        added: &BTreeMap<String, BTreeSet<u64>>,
    ) -> CallGraph {
        let id = |i: usize| {
            let k = &self.known[i];
            format!("{}:{}", k.path, k.function.start)
        };
        let neighbours = |i: usize| {
            self.calls
                .get(&i)
                .into_iter()
                .flatten()
                .chain(self.callers.get(&i).into_iter().flatten())
                .copied()
                .collect::<Vec<usize>>()
        };
        let centre: BTreeSet<usize> = changed
            .iter()
            .filter_map(|(path, f)| self.index_of(path, f))
            .collect();
        let full: BTreeSet<usize> = centre
            .iter()
            .flat_map(|i| std::iter::once(*i).chain(neighbours(*i)))
            .collect();
        let named: BTreeSet<usize> = full
            .iter()
            .flat_map(|i| neighbours(*i))
            .filter(|i| !full.contains(i))
            .collect();
        let mut functions: Vec<CallNode> = full
            .iter()
            .chain(&named)
            .map(|i| {
                let k = &self.known[*i];
                let f = &k.function;
                let whole = full.contains(i);
                let ids = |of: Option<&BTreeSet<usize>>| -> Vec<String> {
                    if whole {
                        of.into_iter().flatten().map(|j| id(*j)).collect()
                    } else {
                        Vec::new()
                    }
                };
                CallNode {
                    id: id(*i),
                    path: k.path.clone(),
                    name: f.name.clone(),
                    start: f.start,
                    end: f.end,
                    direct: !k.effects.is_empty(),
                    effects: k.effects.clone(),
                    sites: if whole { k.sites.clone() } else { Vec::new() },
                    changed: added
                        .get(&k.path)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|l| f.start <= *l && *l <= f.end)
                        .collect(),
                    lines: whole.then(|| tree.get(&k.path)).flatten().map(|text| {
                        text.lines()
                            .skip(f.start.saturating_sub(1) as usize)
                            .take((f.end + 1).saturating_sub(f.start) as usize)
                            .map(str::to_owned)
                            .collect()
                    }),
                    calls: ids(self.calls.get(i)),
                    callers: ids(self.callers.get(i)),
                }
            })
            .collect();
        functions.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
        CallGraph { functions }
    }

    /// For the function `f` of the file at `path`: the functions it
    /// calls, and its steps to an effect.
    pub fn reach(&self, path: &str, f: &crate::syntax::Function) -> (Vec<Callee>, Vec<Step>) {
        let Some(at) = self.index_of(path, f) else {
            return (Vec::new(), Vec::new());
        };
        let callee = |i: &usize| Callee {
            path: self.known[*i].path.clone(),
            function: self.known[*i].function.name.clone(),
        };
        let step = |direction: &'static str, i: &usize| {
            let k = &self.known[*i];
            (!k.effects.is_empty()).then(|| Step {
                direction,
                path: k.path.clone(),
                function: k.function.name.clone(),
                effects: k.effects.clone(),
            })
        };
        let calls: BTreeSet<Callee> = self
            .calls
            .get(&at)
            .into_iter()
            .flatten()
            .map(callee)
            .collect();
        let mut steps: BTreeSet<Step> = BTreeSet::new();
        steps.extend(
            self.calls
                .get(&at)
                .into_iter()
                .flatten()
                .filter_map(|i| step("triggers", i)),
        );
        steps.extend(
            self.callers
                .get(&at)
                .into_iter()
                .flatten()
                .filter_map(|i| step("feeds", i)),
        );
        (calls.into_iter().collect(), steps.into_iter().collect())
    }
}

/// How closely one changed file should be read: where to look first.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct Risk {
    /// The level that stands: the judgement's where one is recorded,
    /// and the computed one otherwise.
    pub level: RiskLevel,
    /// What `ris` works out from the file's signals, with no model.
    pub computed: Computed,
    /// What an auditor recorded for the file in this phase, the latest
    /// if more than one; null when none has.
    pub judged: Option<Judged>,
}

/// The level the signals give, and the signals that gave it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct Computed {
    pub level: RiskLevel,
    /// The signals that count toward the level, in a fixed order; empty
    /// when none does.
    pub signals: Vec<Signal>,
}

/// One signal that counts toward a computed level.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct Signal {
    /// `outside`, `contract`, `diff` or `dependents`; for a function,
    /// also `reach`.
    pub signal: &'static str,
    /// The signal in words, with its numbers.
    pub reason: String,
}

/// An auditor's judgement of one file's change.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct Judged {
    pub level: RiskLevel,
    /// One line of why.
    pub why: String,
    /// The auditor seat that recorded it.
    pub seat: String,
}

/// A diff of this many lines, added plus removed, counts toward the
/// level, and one of [`LARGE_DIFF`] counts twice.
const SIZEABLE_DIFF: u64 = 50;
const LARGE_DIFF: u64 = 200;
/// This many dependents count toward the level, and [`MANY_DEPENDENTS`]
/// count twice.
const SOME_DEPENDENTS: usize = 3;
const MANY_DEPENDENTS: usize = 8;

/// The level a changed file's signals give. Each signal adds points:
/// one for touching the outside, two for being a contract, one or two
/// for the size of the diff, one or two for how many files depend on
/// it. Two points are medium and four are high, so one mild signal
/// alone (touching the outside, as most files of a command-line tool
/// do) stays low, a contract or a large change is at least medium, and
/// high takes signals together.
pub fn computed_risk(
    external: &[&'static str],
    contract: bool,
    lines: u64,
    dependents: usize,
) -> Computed {
    let (points, signals) = tally(external, contract, lines, dependents);
    Computed {
        level: level_of(points),
        signals,
    }
}

/// A changed function's level: the signals a file's would have from its
/// effects and its change, and one point more for being one step from
/// an effect (`steps`), as a call that triggers one or feeds one.
pub fn function_risk(effects: &[&'static str], lines: u64, steps: &[Step]) -> Computed {
    let (mut points, mut signals) = tally(effects, false, lines, 0);
    if !steps.is_empty() {
        points += 1;
        let said: Vec<String> = steps
            .iter()
            .map(|s| {
                format!(
                    "{} {} via {}",
                    s.direction,
                    s.effects.join(", "),
                    s.function
                )
            })
            .collect();
        signals.push(Signal {
            signal: "reach",
            reason: format!("one step from an effect: {}", said.join("; ")),
        });
    }
    Computed {
        level: level_of(points),
        signals,
    }
}

/// Two points are medium and four are high.
fn level_of(points: u32) -> RiskLevel {
    match points {
        0 | 1 => RiskLevel::Low,
        2 | 3 => RiskLevel::Medium,
        _ => RiskLevel::High,
    }
}

/// The points the signals add up to, and the signals that counted.
fn tally(
    external: &[&'static str],
    contract: bool,
    lines: u64,
    dependents: usize,
) -> (u32, Vec<Signal>) {
    let mut points = 0;
    let mut signals = Vec::new();
    let mut count = |n: u32, signal: &'static str, reason: String| {
        points += n;
        signals.push(Signal { signal, reason });
    };
    if !external.is_empty() {
        count(
            1,
            "outside",
            format!("touches the outside: {}", external.join(", ")),
        );
    }
    if contract {
        count(2, "contract", "a contract or format".to_owned());
    }
    if lines >= LARGE_DIFF {
        count(2, "diff", format!("a large change: {lines} lines"));
    } else if lines >= SIZEABLE_DIFF {
        count(1, "diff", format!("a sizeable change: {lines} lines"));
    }
    if dependents >= MANY_DEPENDENTS {
        count(
            2,
            "dependents",
            format!("{dependents} other files depend on it"),
        );
    } else if dependents >= SOME_DEPENDENTS {
        count(
            1,
            "dependents",
            format!("{dependents} other files depend on it"),
        );
    }
    (points, signals)
}

/// A level as the document and the text spell it.
pub fn level_word(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
    }
}

/// One connection, from the file that names the other.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// The kind of line that makes it: `mod` or `use` for Rust.
    pub kind: &'static str,
}

/// What is compared: a phase's tip against where it left the main line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub phase: Option<PhaseId>,
    /// The commit the phase started from; `None` when it is not known,
    /// and then no file carries a diff.
    pub base: Option<String>,
    /// The commit whose files are the nodes.
    pub tip: String,
    /// What `tip` was named by: a branch, or how the commit was found.
    pub tip_ref: String,
}

/// A node's weight: one for being there, the diff's size on a log
/// scale so a large change does not swamp the picture, and two for
/// touching the outside or being a contract.
pub fn weight(added: u64, removed: u64, outside: bool) -> f64 {
    let w = 1.0 + ((1 + added + removed) as f64).ln() + if outside { 2.0 } else { 0.0 };
    (w * 100.0).round() / 100.0
}

/// Whether `path` matches `pattern`: `/` separates segments, `*`
/// matches within a segment and `**` matches any number of segments.
pub fn glob(pattern: &str, path: &str) -> bool {
    fn segment(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => segment(&p[1..], s) || (!s.is_empty() && segment(p, &s[1..])),
            (Some(a), Some(b)) if a == b => segment(&p[1..], &s[1..]),
            _ => false,
        }
    }
    fn segments(p: &[&str], s: &[&str]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(&"**"), _) => segments(&p[1..], s) || (!s.is_empty() && segments(p, &s[1..])),
            (Some(a), Some(b)) if segment(a.as_bytes(), b.as_bytes()) => segments(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    segments(&p, &s)
}

/// The graph document from its parts. `tree` is the tip's files,
/// `diff` each changed path's status and line counts, `config` the
/// configured patterns. Pure, so a test builds it without git.
///
/// A test file (a `tests` pattern names it, and no `contracts` one) is
/// left out: no node and no edge, only a count in the totals.
///
/// `judged` is what auditors recorded for the phase, by path; one for
/// a file the phase did not change is not carried.
pub fn document(
    range: &Range,
    paths: &[String],
    tree: &Tree,
    diff: &BTreeMap<String, (&'static str, u64, u64)>,
    config: &crate::config::MapConfig,
    judged: &BTreeMap<String, Judged>,
) -> crate::schema::Map {
    let is_contract = |path: &str| config.contracts.iter().any(|c| glob(c, path));
    let is_test = |path: &str| !is_contract(path) && config.tests.iter().any(|t| glob(t, path));
    let on_map = |path: &str| language_of(path).is_some() || is_contract(path);
    let is_node = |path: &str| on_map(path) && !is_test(path);
    let mut scans: BTreeMap<String, Scan> = BTreeMap::new();
    for language in LANGUAGES {
        scans.extend(language.scan(tree));
    }
    // The tip's files, then what the phase deleted.
    let mut names: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    names.extend(diff.keys().map(String::as_str));
    let mut tests = crate::schema::MapTests::default();
    for path in names.iter().filter(|p| on_map(p) && is_test(p)) {
        tests.files += 1;
        if let Some((_, added, removed)) = diff.get(*path) {
            tests.changed += 1;
            tests.added += added;
            tests.removed += removed;
        }
    }
    let mut nodes: Vec<Node> = names
        .into_iter()
        .filter(|p| is_node(p))
        .map(|path| {
            let (status, added, removed) = diff.get(path).copied().unwrap_or(("unchanged", 0, 0));
            let external: Vec<&'static str> = scans
                .get(path)
                .map(|s| s.external.iter().copied().collect())
                .unwrap_or_default();
            let contract = is_contract(path);
            Node {
                path: path.to_owned(),
                language: language_of(path),
                status,
                added,
                removed,
                weight: weight(added, removed, contract || !external.is_empty()),
                external,
                contract,
                dependents: 0,
                risk: None,
                functions: None,
                marks: None,
            }
        })
        .collect();
    let mut edges = BTreeSet::new();
    {
        let known: BTreeSet<&str> = nodes.iter().map(|n| n.path.as_str()).collect();
        for (from, scan) in &scans {
            for (to, kind) in &scan.edges {
                if from != to && known.contains(from.as_str()) && known.contains(to.as_str()) {
                    edges.insert(Edge {
                        from: from.clone(),
                        to: to.clone(),
                        kind,
                    });
                }
            }
        }
    }
    // A file's dependents are the files with an edge to it, each once
    // however many kinds of line name it.
    let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in &edges {
        dependents.entry(&e.to).or_default().insert(&e.from);
    }
    for n in &mut nodes {
        n.dependents = dependents.get(n.path.as_str()).map_or(0, BTreeSet::len);
        if n.status != "unchanged" {
            let computed =
                computed_risk(&n.external, n.contract, n.added + n.removed, n.dependents);
            let judged = judged.get(&n.path).cloned();
            n.risk = Some(Risk {
                level: judged.as_ref().map_or(computed.level, |j| j.level),
                computed,
                judged,
            });
        }
    }
    let changed: Vec<&Node> = nodes.iter().filter(|n| n.status != "unchanged").collect();
    let ranked = ranked(&nodes);
    crate::schema::Map {
        phase: range.phase,
        base: range.base.clone(),
        tip: range.tip.clone(),
        tip_ref: range.tip_ref.clone(),
        totals: crate::schema::MapTotals {
            files: nodes.len(),
            changed: changed.len(),
            added: changed.iter().map(|n| n.added).sum(),
            removed: changed.iter().map(|n| n.removed).sum(),
            outside: nodes
                .iter()
                .filter(|n| n.contract || !n.external.is_empty())
                .count(),
            tests,
        },
        nodes,
        ranked,
        edges: edges.into_iter().collect(),
        diffs: BTreeMap::new(),
        call_graph: CallGraph::default(),
    }
}

/// The changed files' paths in the order to read them, where to look
/// first: the highest risk level that stands, then the heaviest, then
/// by path.
pub fn ranked(nodes: &[Node]) -> Vec<String> {
    let mut changed: Vec<&Node> = nodes.iter().filter(|n| n.status != "unchanged").collect();
    changed.sort_by(|a, b| {
        let level = |n: &Node| n.risk.as_ref().map(|r| r.level);
        level(b)
            .cmp(&level(a))
            .then(b.weight.total_cmp(&a.weight))
            .then(a.path.cmp(&b.path))
    });
    changed.into_iter().map(|n| n.path.clone()).collect()
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn git_line(dir: &Path, args: &[&str]) -> Result<String, String> {
    git(dir, args).map(|b| String::from_utf8_lossy(&b).trim().to_owned())
}

/// The subject `ris phase close` gives the closing merge of `phase`,
/// up to the goal: how a closed phase's merge is found on the main
/// line.
pub fn closing_subject(phase: PhaseId) -> String {
    format!("chore: close {phase}: ")
}

/// What to compare for `phase`, or for the open phase, or with none
/// open for the last one closed.
///
/// While the phase's branch exists, it against where it left the main
/// line. An open phase with no branch of its own (phase branches off)
/// is the caller's checkout against the main line. A closed phase whose
/// branch is gone is its closing merge against the main line before it.
pub fn range(p: &Project, l: &Loaded, phase: Option<PhaseId>) -> Result<Range, CliError> {
    let dir = &p.work;
    if git(dir, &["rev-parse", "--git-dir"]).is_err() {
        return Err(CliError::Other(
            "ris map reads the files and the diff from git, and this project is not a git repository"
                .into(),
        ));
    }
    let phase = match phase {
        Some(id) => Some(l.state.phases.get(&id).ok_or_else(|| {
            CliError::Other(format!(
                "no phase {id}\n  next: `ris report` names the phases"
            ))
        })?),
        None => l
            .state
            .open_phase()
            .or_else(|| l.state.phases.values().next_back()),
    };
    let main = &p.config.main_line;
    let head = |name: &str| git_line(dir, &["rev-parse", "--verify", "--quiet", name]);
    let fork = |tip: &str| git_line(dir, &["merge-base", main, tip]).ok();
    let here = || {
        head("HEAD^{commit}").map_err(|_| {
            CliError::Other(
                "the repository has no commit yet, so there are no files to map\n  \
                 next: commit, then `ris map`"
                    .into(),
            )
        })
    };
    let Some(phase) = phase else {
        return Ok(Range {
            phase: None,
            base: None,
            tip: here()?,
            tip_ref: "HEAD".to_owned(),
        });
    };
    let branch = format!("phase/{}", phase.id);
    if let Ok(tip) = head(&format!("refs/heads/{branch}^{{commit}}")) {
        return Ok(Range {
            phase: Some(phase.id),
            base: fork(&tip),
            tip,
            tip_ref: branch,
        });
    }
    if matches!(phase.state, PhaseState::Open) {
        let tip = here()?;
        return Ok(Range {
            phase: Some(phase.id),
            base: fork(&tip),
            tip,
            tip_ref: "HEAD".to_owned(),
        });
    }
    // Closed, and its branch merged and deleted: the closing merge.
    let subject = closing_subject(phase.id);
    let log = git_line(
        dir,
        &["log", "--first-parent", "--merges", "--format=%H %s", main],
    )
    .unwrap_or_default();
    let merge = log
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(_, s)| s.starts_with(&subject))
        .map(|(sha, _)| sha.to_owned());
    match merge {
        Some(tip) => Ok(Range {
            phase: Some(phase.id),
            base: head(&format!("{tip}^1")).ok(),
            tip,
            tip_ref: format!("the closing merge of {} on {main}", phase.id),
        }),
        None => Err(CliError::Other(format!(
            "{} is closed and neither its branch {branch} nor its closing merge on {main} is in this repository, \
             so its change set cannot be read",
            phase.id
        ))),
    }
}

/// The contents of `paths` at `commit`, through one `git cat-file`.
/// A path that is not a blob there, or is not UTF-8, is left out.
fn read_blobs(dir: &Path, commit: &str, paths: &[&str]) -> Result<Tree, CliError> {
    let mut tree = Tree::new();
    if paths.is_empty() {
        return Ok(tree);
    }
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped");
    let request: String = paths.iter().map(|p| format!("{commit}:{p}\n")).collect();
    // Written from its own thread: git answers as it reads, and both
    // pipes would fill otherwise.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(request.as_bytes());
    });
    let mut out = Vec::new();
    child.stdout.take().expect("piped").read_to_end(&mut out)?;
    let _ = writer.join();
    child.wait()?;
    let mut rest = &out[..];
    for path in paths {
        let Some(end) = rest.iter().position(|b| *b == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        // `<sha> <type> <size>`, or `<name> missing`.
        let mut parts = header.rsplitn(3, ' ');
        let size = parts.next().and_then(|s| s.parse::<usize>().ok());
        let kind = parts.next();
        let Some(size) = size else { continue };
        if rest.len() < size + 1 {
            break;
        }
        if kind == Some("blob")
            && let Ok(text) = std::str::from_utf8(&rest[..size])
        {
            tree.insert((*path).to_owned(), text.to_owned());
        }
        rest = &rest[size + 1..];
    }
    Ok(tree)
}

/// What auditors recorded for `phase`, by path: the latest judgement of
/// each file. One whose text has been removed is left out, since the
/// file it was about is in that text.
pub fn judgements(l: &Loaded, phase: Option<PhaseId>) -> BTreeMap<String, Judged> {
    let mut out = BTreeMap::new();
    let seqs = phase.and_then(|id| l.state.risks.get(&id));
    for seq in seqs.into_iter().flatten() {
        let Some(Structure::RiskJudged { seat, level, .. }) = l.event(*seq).map(|e| &e.structure)
        else {
            continue;
        };
        let (Some(path), Some(why)) = (l.log_text(*seq, "path"), l.log_text(*seq, "why")) else {
            continue;
        };
        out.insert(
            path.to_owned(),
            Judged {
                level: *level,
                why: why.to_owned(),
                seat: seat.0.clone(),
            },
        );
    }
    out
}

/// The graph document for `range`, read from git, with each changed
/// file's diff.
pub fn build(p: &Project, l: &Loaded, range: &Range) -> Result<crate::schema::Map, CliError> {
    build_in(&p.work, &p.config.map, &judgements(l, range.phase), range)
}

/// [`build`] over the repository at `dir`, given what was `judged`:
/// all of it is read from git, and none from the store.
fn build_in(
    dir: &Path,
    config: &crate::config::MapConfig,
    judged: &BTreeMap<String, Judged>,
    range: &Range,
) -> Result<crate::schema::Map, CliError> {
    let (mut map, tree) = read(dir, config, judged, range)?;
    if let Some(base) = &range.base {
        for node in map.nodes.iter().filter(|n| n.status != "unchanged") {
            map.diffs.insert(
                node.path.clone(),
                unified_diff(dir, base, &range.tip, &node.path)?,
            );
        }
    }
    annotate(&mut map, &tree, config);
    Ok(map)
}

/// The graph document for `range` without its diffs: the nodes, the
/// edges and the totals.
pub fn survey(p: &Project, l: &Loaded, range: &Range) -> Result<crate::schema::Map, CliError> {
    Ok(read(&p.work, &p.config.map, &judgements(l, range.phase), range)?.0)
}

/// [`survey`], with the files it read at the tip.
fn read(
    dir: &Path,
    config: &crate::config::MapConfig,
    judged: &BTreeMap<String, Judged>,
    range: &Range,
) -> Result<(crate::schema::Map, Tree), CliError> {
    let listed =
        git(dir, &["ls-tree", "-r", "-z", "--name-only", &range.tip]).map_err(CliError::Other)?;
    let paths: Vec<String> = listed
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        // A path with a newline cannot be asked of cat-file by line.
        .filter(|s| !s.contains('\n'))
        .collect();
    let manifests: BTreeSet<&str> = LANGUAGES
        .iter()
        .flat_map(|l| l.manifests())
        .copied()
        .collect();
    let wanted: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| {
            language_of(path).is_some()
                || manifests.contains(path.rsplit('/').next().unwrap_or(path))
        })
        .collect();
    let tree = read_blobs(dir, &range.tip, &wanted)?;
    let mut diff: BTreeMap<String, (&'static str, u64, u64)> = BTreeMap::new();
    if let Some(base) = &range.base {
        let records = |args: &[&str]| -> Result<Vec<String>, CliError> {
            let mut all = vec!["diff", "--no-renames", "-z"];
            all.extend_from_slice(args);
            all.extend_from_slice(&[base.as_str(), range.tip.as_str()]);
            Ok(git(dir, &all)
                .map_err(CliError::Other)?
                .split(|b| *b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect())
        };
        // `<status>`, then the path, each its own record.
        for pair in records(&["--name-status"])?.chunks(2) {
            if let [status, path] = pair {
                let status = match status.as_str() {
                    "A" => "added",
                    "D" => "deleted",
                    _ => "modified",
                };
                diff.insert(path.clone(), (status, 0, 0));
            }
        }
        // `<added>\t<removed>\t<path>`; a binary file's counts are `-`.
        for record in records(&["--numstat"])? {
            let mut parts = record.splitn(3, '\t');
            let added = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let removed = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            if let Some(entry) = parts.next().and_then(|path| diff.get_mut(path)) {
                entry.1 = added;
                entry.2 = removed;
            }
        }
    }
    let map = document(range, &paths, &tree, &diff, config, judged);
    Ok((map, tree))
}

/// One file's unified diff between two commits, as git writes it: the
/// file header, then each hunk. The flags fix its form whatever the
/// reader's git configuration says, so the same range gives the same
/// text everywhere. Bytes that are not UTF-8 are replaced.
fn unified_diff(dir: &Path, base: &str, tip: &str, path: &str) -> Result<String, CliError> {
    let out = git(
        dir,
        &[
            "--literal-pathspecs",
            "diff",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            base,
            tip,
            "--",
            path,
        ],
    )
    .map_err(CliError::Other)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The page's template: the vendored d3 build goes in at `/*D3*/` and
/// the document at `/*DATA*/`.
const PAGE: &str = include_str!("../assets/map.html");
/// d3's minified build, vendored so the page opens with no network.
const D3: &str = include_str!("../assets/d3/d3.v7.9.0.min.js");

/// One self-contained page drawing `doc`. The document is embedded
/// whole, as JSON in a script element a renderer reads back; `<` is
/// written as its escape so no text in it can end the element.
pub fn page(doc: &serde_json::Value) -> String {
    let data = doc.to_string().replace('<', "\\u003c");
    PAGE.replacen("/*D3*/", D3, 1)
        .replacen("/*DATA*/", &data, 1)
}

/// The one line that says what the map left out as tests; the page
/// writes the same words.
pub fn tests_line(t: &crate::schema::MapTests) -> String {
    format!(
        "{} test file(s) left off the map: {} changed (+{} -{})",
        t.files, t.changed, t.added, t.removed
    )
}

/// The page's fixed file inside the `.osiris` directory, which git
/// ignores: where `ris map` with no flags writes it, replacing the
/// last one.
pub const PAGE_FILE: &str = "map.html";

/// What `ris map` does with the page.
pub enum Show<'a> {
    /// Nothing: the text form alone.
    Text,
    /// Nothing, and the text lists each changed file with its reasons
    /// (`--list`).
    List,
    /// Write it to the file named (`--html`).
    Write(&'a Path),
    /// Write it to `file`, which is said as `shown`, and open it in the
    /// browser with `opener`.
    Open {
        file: &'a Path,
        shown: &'a str,
        opener: &'a Opener,
    },
}

/// Whether `ris map` writes the page to its fixed file and opens it:
/// only when the person runs it at a terminal and asks for neither
/// JSON nor a file of their own. A seat never opens anything on the
/// person's screen, and output that is piped or captured is being read
/// by a program, not by someone who wants a browser window.
pub fn opens_page(actor: &Actor, json: bool, html: bool, terminal: bool) -> bool {
    matches!(actor, Actor::Person) && !json && !html && terminal
}

/// Which output a `ris map` gives, without its page or file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// The page, written to the file `--html` names.
    Write,
    /// The page, written to its fixed file and opened in the browser.
    Open,
    /// The ranked list with each file's reasons (`--list`).
    List,
    /// The text form.
    Text,
}

/// What `ris map` gives: `--html` writes the page there; otherwise the
/// page is opened only where [`opens_page`] says, which is never with
/// `--list`; `--list` prints the list; and anything else is the text.
pub fn form(actor: &Actor, json: bool, html: bool, list: bool, terminal: bool) -> Form {
    if html {
        Form::Write
    } else if !list && opens_page(actor, json, false, terminal) {
        Form::Open
    } else if list {
        Form::List
    } else {
        Form::Text
    }
}

/// How a file is opened in the browser: a program, and the arguments
/// that go before the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opener {
    pub program: String,
    pub args: Vec<String>,
}

impl Opener {
    /// The platform's own way to open a file with whatever handles it,
    /// which for a page is the person's browser.
    pub fn platform() -> Self {
        let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
            ("open", &[])
        } else if cfg!(windows) {
            // `start` takes a quoted first argument as a window title.
            ("cmd", &["/C", "start", ""])
        } else {
            ("xdg-open", &[])
        };
        Self {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    /// Open `file`, or say in a few words why it did not open: the
    /// program could not be started, or it reported a failure.
    pub fn open(&self, file: &Path) -> Result<(), String> {
        let status = Command::new(&self.program)
            .args(&self.args)
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("`{}` could not be started: {}", self.program, e.kind()))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`{}` reported a failure", self.program))
        }
    }
}

/// `ris map`: the document, and in text its totals and the changed
/// files, each with its risk level, the highest risk first and within a
/// level the heaviest. `show` says what becomes of the page. A browser
/// that does not open is said, with the file to open by hand, and is
/// not an error: the page is written either way.
pub fn output(map: crate::schema::Map, show: Show) -> Result<Output, CliError> {
    let doc = serde_json::to_value(&map).expect("the map serialises");
    let t = &doc["totals"];
    let mut text = format!(
        "{}: {} of {} source file(s) changed (+{} -{}), {} touch the outside or are contracts\n",
        doc["phase"].as_str().unwrap_or("no phase"),
        t["changed"],
        t["files"],
        t["added"],
        t["removed"],
        t["outside"],
    );
    text.push_str(&format!("{}\n", tests_line(&map.totals.tests)));
    if doc["base"].is_null() {
        text.push_str(
            "where the work left the main line is not known, so no file carries a diff\n",
        );
    }
    // Where to look first, in the document's own order.
    let by_path: BTreeMap<&str, &serde_json::Value> = doc["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n["path"].as_str()?, n)))
        .collect();
    let changed = doc["ranked"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| by_path.get(p.as_str()?).copied());
    let listing = matches!(show, Show::List);
    for n in changed {
        let mut marks: Vec<&str> = n["external"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str())
            .collect();
        if n["contract"] == true {
            marks.push("contract");
        }
        text.push_str(&format!(
            "  {:<6} {:>6} {:<6} {}{}{}\n",
            n["risk"]["level"].as_str().unwrap_or(""),
            format!("+{}", n["added"]),
            format!("-{}", n["removed"]),
            n["path"].as_str().unwrap_or(""),
            if n["status"] == "modified" {
                String::new()
            } else {
                format!(" ({})", n["status"].as_str().unwrap_or(""))
            },
            if marks.is_empty() {
                String::new()
            } else {
                format!("  [{}]", marks.join(", "))
            },
        ));
        let judged = &n["risk"]["judged"];
        let computed = &n["risk"]["computed"];
        if listing {
            text.push_str(&reasons(judged, computed));
            text.push_str(&marked_lines(&n["marks"]));
            text.push_str(&one_step(&n["functions"]));
        } else if !judged.is_null() {
            text.push_str(&format!(
                "         judged {} by {}: {} (computed: {})\n",
                judged["level"].as_str().unwrap_or(""),
                judged["seat"].as_str().unwrap_or(""),
                judged["why"].as_str().unwrap_or(""),
                computed["level"].as_str().unwrap_or(""),
            ));
        }
    }
    match show {
        Show::List => text.push_str(
            "  next: `ris map` draws the same files as a graph beside this list, \
             each with its diff",
        ),
        Show::Write(path) => {
            std::fs::write(path, page(&doc))?;
            text.push_str(&format!(
                "wrote {}\n  next: open it in a browser; it needs no network",
                path.display()
            ));
        }
        Show::Open {
            file,
            shown,
            opener,
        } => {
            std::fs::write(file, page(&doc))?;
            match opener.open(file) {
                Ok(()) => text.push_str(&format!("wrote {shown} and opened it in the browser")),
                Err(why) => text.push_str(&format!(
                    "wrote {shown}; it did not open in the browser: {why}\n  \
                     next: open that file in a browser yourself; it needs no network"
                )),
            }
        }
        Show::Text => text.push_str(
            "  next: `ris map --html map.html` draws it as a page; \
             the person's `ris map` at a terminal opens it",
        ),
    }
    Ok(Output::of(text, &map))
}

/// A changed file's marked lines, indented under its reasons: each line
/// that touches the outside, the function it sits in and that
/// function's level, which is the line's.
fn marked_lines(marks: &serde_json::Value) -> String {
    let mut out = String::new();
    for m in marks.as_array().into_iter().flatten() {
        out.push_str(&format!(
            "         line {}: {}",
            m["line"],
            m["kind"].as_str().unwrap_or("")
        ));
        if let Some(f) = m["function"].as_str() {
            out.push_str(&format!(" in {f}"));
        }
        if let Some(level) = m["risk"].as_str() {
            out.push_str(&format!(" ({level})"));
        }
        out.push('\n');
    }
    out
}

/// A changed file's functions one step from an effect, indented under
/// its marked lines: each with its direction, the effect and the
/// function it reaches, and that function's file.
fn one_step(functions: &serde_json::Value) -> String {
    let mut out = String::new();
    for f in functions.as_array().into_iter().flatten() {
        for s in f["steps"].as_array().into_iter().flatten() {
            let effects: Vec<&str> = s["effects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str())
                .collect();
            out.push_str(&format!(
                "         {} {} {} via {} ({})\n",
                f["name"].as_str().unwrap_or(""),
                s["direction"].as_str().unwrap_or(""),
                effects.join(", "),
                s["function"].as_str().unwrap_or(""),
                s["path"].as_str().unwrap_or(""),
            ));
        }
    }
    out
}

/// A changed file's reasons, indented under its row: the judgement and
/// its why where one is recorded, then the computed level with the
/// signals that gave it, as the page's pane says them.
fn reasons(judged: &serde_json::Value, computed: &serde_json::Value) -> String {
    let mut out = String::new();
    if !judged.is_null() {
        out.push_str(&format!(
            "         judged {} by {}: {}\n",
            judged["level"].as_str().unwrap_or(""),
            judged["seat"].as_str().unwrap_or(""),
            judged["why"].as_str().unwrap_or(""),
        ));
    }
    let signals: Vec<&str> = computed["signals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["reason"].as_str())
        .collect();
    out.push_str(&format!(
        "         computed {}: {}\n",
        computed["level"].as_str().unwrap_or(""),
        if signals.is_empty() {
            "no signal counts".to_owned()
        } else {
            signals.join("; ")
        }
    ));
    out
}

/// Rust: `mod` lines, `use` lines and qualified paths, by scanning.
pub struct Rust;

/// Paths that mean a Rust file touches the outside only when they are
/// the whole path: the process module imported as itself. A longer
/// path through it may only read the process's own id or end it.
const RUST_OUTSIDE_WHOLE: &[(&[&str], &str)] = &[
    (&["std", "process"], "process"),
    (&["tokio", "process"], "process"),
];

/// Path prefixes that mean a Rust file touches the outside; joined by
/// `::`, the shipped effect patterns for Rust inside a file.
pub(crate) const RUST_OUTSIDE: &[(&[&str], &str)] = &[
    (&["std", "process", "Command"], "process"),
    (&["std", "process", "Child"], "process"),
    (&["std", "process", "Stdio"], "process"),
    (&["tokio", "process", "Command"], "process"),
    (&["tokio", "process", "Child"], "process"),
    (&["std", "fs"], "filesystem"),
    (&["tokio", "fs"], "filesystem"),
    (&["fs_err"], "filesystem"),
    (&["tempfile"], "filesystem"),
    (&["walkdir"], "filesystem"),
    (&["std", "net"], "network"),
    (&["tokio", "net"], "network"),
    (&["reqwest"], "network"),
    (&["hyper"], "network"),
    (&["ureq"], "network"),
    (&["tonic"], "network"),
    (&["axum"], "network"),
    (&["tungstenite"], "network"),
    (&["rusqlite"], "database"),
    (&["sqlx"], "database"),
    (&["diesel"], "database"),
    (&["postgres"], "database"),
    (&["tokio_postgres"], "database"),
    (&["redis"], "database"),
    (&["rocksdb"], "database"),
    (&["sled"], "database"),
    (&["mongodb"], "database"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    /// `::`
    Sep,
    Punct(char),
}

/// Rust source as identifiers and punctuation, with comments, strings
/// and character literals dropped.
fn tokens(text: &str) -> Vec<Tok> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // Skip a string body from just after its opening quote.
    let string = |mut i: usize| {
        while i < c.len() && c[i] != '"' {
            i += if c[i] == '\\' { 2 } else { 1 };
        }
        i + 1
    };
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while i < c.len() && depth > 0 {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if ch == '"' {
            i = string(i + 1);
        } else if ch == '\'' {
            // A character literal, or a lifetime's tick.
            if next == Some('\\') {
                // The tick, the backslash and what it escapes, then
                // on to the closing tick ('\u{1F}' is longer).
                i += 3;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let word: String = c[start..i].iter().collect();
            // A raw string: r"..", r#".."#, br#".."#.
            if word == "r" || word == "br" {
                let mut hashes = 0;
                while c.get(i + hashes) == Some(&'#') {
                    hashes += 1;
                }
                if c.get(i + hashes) == Some(&'"') {
                    i += hashes + 1;
                    while i < c.len() {
                        if c[i] == '"' && (1..=hashes).all(|h| c.get(i + h) == Some(&'#')) {
                            break;
                        }
                        i += 1;
                    }
                    i += hashes + 1;
                    continue;
                }
                if word == "r" && hashes == 1 {
                    // A raw identifier, r#name: the name follows.
                    i += 1;
                    continue;
                }
            }
            out.push(Tok::Ident(word));
        } else if ch == ':' && next == Some(':') {
            out.push(Tok::Sep);
            i += 2;
        } else {
            if !ch.is_whitespace() {
                out.push(Tok::Punct(ch));
            }
            i += 1;
        }
    }
    out
}

/// What a Rust file names: its `mod` lines, and every path of two or
/// more segments, each with how many inline modules enclose it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Names {
    mods: Vec<String>,
    paths: Vec<(Vec<String>, usize)>,
}

/// One `use` tree from `t[i]`, each leaf pushed under `prefix`; the
/// index after it. Stops before the `,`, `}` or `;` that ends it.
fn use_tree(t: &[Tok], mut i: usize, mut prefix: Vec<String>, out: &mut Vec<Vec<String>>) -> usize {
    loop {
        match t.get(i) {
            Some(Tok::Ident(word)) if word == "as" => i += 2,
            Some(Tok::Ident(word)) => {
                prefix.push(word.clone());
                i += 1;
            }
            Some(Tok::Sep) => i += 1,
            Some(Tok::Punct('{')) => {
                i += 1;
                loop {
                    match t.get(i) {
                        Some(Tok::Punct('}')) => {
                            i += 1;
                            break;
                        }
                        Some(Tok::Punct(',')) => i += 1,
                        None => break,
                        _ => {
                            let after = use_tree(t, i, prefix.clone(), out);
                            // A token no tree takes: step over it.
                            i = if after == i { i + 1 } else { after };
                        }
                    }
                }
                return i;
            }
            Some(Tok::Punct('*')) => {
                out.push(prefix);
                return i + 1;
            }
            _ => {
                out.push(prefix);
                return i;
            }
        }
    }
}

fn names(text: &str) -> Names {
    let t = tokens(text);
    let mut found = Names::default();
    // Brace depths at which an inline module opened.
    let mut inline: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < t.len() {
        match &t[i] {
            Tok::Punct('{') => {
                depth += 1;
                i += 1;
            }
            Tok::Punct('}') => {
                if inline.last() == Some(&depth) {
                    inline.pop();
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            Tok::Ident(word) if word == "mod" => match (t.get(i + 1), t.get(i + 2)) {
                (Some(Tok::Ident(name)), Some(Tok::Punct(';'))) => {
                    if inline.is_empty() {
                        found.mods.push(name.clone());
                    }
                    i += 3;
                }
                (Some(Tok::Ident(_)), Some(Tok::Punct('{'))) => {
                    depth += 1;
                    inline.push(depth);
                    i += 3;
                }
                _ => i += 1,
            },
            // `use<..>` is a capture list in a type, not an import.
            Tok::Ident(word) if word == "use" && t.get(i + 1) != Some(&Tok::Punct('<')) => {
                let mut leaves = Vec::new();
                i = use_tree(&t, i + 1, Vec::new(), &mut leaves);
                found.paths.extend(
                    leaves
                        .into_iter()
                        .filter(|p| !p.is_empty())
                        .map(|p| (p, inline.len())),
                );
            }
            Tok::Ident(first) => {
                let mut path = vec![first.clone()];
                i += 1;
                while let (Some(Tok::Sep), Some(Tok::Ident(next))) = (t.get(i), t.get(i + 1)) {
                    path.push(next.clone());
                    i += 2;
                }
                if path.len() > 1 {
                    found.paths.push((path, inline.len()));
                }
            }
            _ => i += 1,
        }
    }
    found
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// A Rust package in the tree.
struct Package {
    /// Its directory; empty at the repository root.
    dir: String,
    /// The file `its_name::` reaches: the library's root, or the
    /// binary's when it has no library.
    root: Option<String>,
}

impl Rust {
    /// Package names (as code spells them, `_` for `-`) to packages.
    fn packages(tree: &Tree) -> BTreeMap<String, Package> {
        let mut out = BTreeMap::new();
        for (path, text) in tree {
            if path.rsplit('/').next() != Some("Cargo.toml") {
                continue;
            }
            let Ok(manifest) = text.parse::<toml::Table>() else {
                continue;
            };
            let Some(name) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            else {
                continue;
            };
            let lib = manifest.get("lib");
            let name = lib
                .and_then(|l| l.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or(name)
                .replace('-', "_");
            let dir = parent(path).to_owned();
            let root = lib
                .and_then(|l| l.get("path"))
                .and_then(|p| p.as_str())
                .map(|p| join(&dir, p))
                .into_iter()
                .chain([join(&dir, "src/lib.rs"), join(&dir, "src/main.rs")])
                .find(|f| tree.contains_key(f));
            out.insert(name, Package { dir, root });
        }
        out
    }

    /// The directory a file's child modules live in. A crate root, a
    /// `mod.rs` and any file outside a package's `src` keep them beside
    /// itself; `a/b.rs` keeps them in `a/b`.
    fn child_dir(file: &str, src: Option<&str>) -> String {
        let dir = parent(file);
        let name = file.rsplit('/').next().unwrap_or(file);
        let beside = matches!(name, "mod.rs" | "lib.rs" | "main.rs")
            || src.is_none_or(|src| {
                !file.starts_with(&format!("{src}/")) || file.starts_with(&format!("{src}/bin/"))
            });
        if beside {
            dir.to_owned()
        } else {
            join(dir, name.trim_end_matches(".rs"))
        }
    }

    /// The file that is the module whose children live in `dir`.
    fn module_file(dir: &str, tree: &Tree) -> Option<String> {
        [
            format!("{dir}.rs"),
            join(dir, "mod.rs"),
            join(dir, "lib.rs"),
            join(dir, "main.rs"),
        ]
        .into_iter()
        .find(|f| tree.contains_key(f))
    }

    /// Walk `segments` down from `dir` as far as files go: the deepest
    /// module file reached, if any.
    fn descend(dir: &str, segments: &[String], tree: &Tree) -> Option<String> {
        let mut dir = dir.to_owned();
        let mut found = None;
        for s in segments {
            let file = join(&dir, &format!("{s}.rs"));
            let nested = join(&dir, &format!("{s}/mod.rs"));
            if tree.contains_key(&file) {
                found = Some(file);
            } else if tree.contains_key(&nested) {
                found = Some(nested);
            } else {
                break;
            }
            dir = join(&dir, s);
        }
        found
    }
}

impl Language for Rust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn manifests(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn scan(&self, tree: &Tree) -> BTreeMap<String, Scan> {
        let packages = Self::packages(tree);
        let mut out = BTreeMap::new();
        for (file, text) in tree {
            if language_of(file) != Some(self.name()) {
                continue;
            }
            // The package whose directory holds the file, the deepest.
            let package = packages
                .values()
                .filter(|p| p.dir.is_empty() || file.starts_with(&format!("{}/", p.dir)))
                .max_by_key(|p| p.dir.len());
            let src = package.map(|p| join(&p.dir, "src"));
            let here = Self::child_dir(file, src.as_deref());
            // `crate::` is the package's source for a file in it, and
            // the file itself for one that is its own crate (a test, an
            // example, a build script).
            let crate_dir = match &src {
                Some(src)
                    if file.starts_with(&format!("{src}/"))
                        && !file.starts_with(&format!("{src}/bin/")) =>
                {
                    src.clone()
                }
                _ => here.clone(),
            };
            let found = names(text);
            let mut scan = Scan::default();
            for name in &found.mods {
                if let Some(to) = Self::descend(&here, std::slice::from_ref(name), tree) {
                    scan.edges.insert((to, "mod"));
                }
            }
            for (path, inline) in &found.paths {
                for (prefix, kind) in RUST_OUTSIDE {
                    if path.len() >= prefix.len() && path.iter().zip(*prefix).all(|(a, b)| a == b) {
                        scan.external.insert(*kind);
                    }
                }
                for (whole, kind) in RUST_OUTSIDE_WHOLE {
                    if path.iter().map(String::as_str).eq(whole.iter().copied()) {
                        scan.external.insert(*kind);
                    }
                }
                let supers = path.iter().take_while(|s| *s == "super").count();
                let to = match path[0].as_str() {
                    // Inside an inline module `self` is that module,
                    // which is no file.
                    "self" if *inline == 0 => Self::descend(&here, &path[1..], tree),
                    "self" => None,
                    // Each `super` first leaves an inline module, then
                    // climbs a directory.
                    "super" if supers < *inline => None,
                    "super" => {
                        let mut dir = here.clone();
                        for _ in *inline..supers {
                            dir = parent(&dir).to_owned();
                        }
                        Self::descend(&dir, &path[supers..], tree).or_else(|| {
                            (supers > *inline)
                                .then(|| Self::module_file(&dir, tree))
                                .flatten()
                        })
                    }
                    "crate" => Self::descend(&crate_dir, &path[1..], tree)
                        .or_else(|| Self::module_file(&crate_dir, tree)),
                    name => packages.get(name).and_then(|p| {
                        Self::descend(&join(&p.dir, "src"), &path[1..], tree)
                            .or_else(|| p.root.clone())
                    }),
                };
                if let Some(to) = to
                    && to != *file
                {
                    scan.edges.insert((to, "use"));
                }
            }
            out.insert(file.clone(), scan);
        }
        out
    }
}

#[cfg(test)]
#[path = "tests/map.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/map_page.rs"]
mod page_tests;
