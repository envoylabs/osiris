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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use ris_core::ids::PhaseId;
use ris_core::primitives::PhaseState;
use serde::Serialize;
use serde_json::json;

use crate::project::{CliError, Loaded, Project};
use crate::render::Output;

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
#[derive(Clone, Debug, Serialize, PartialEq)]
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
}

/// One connection, from the file that names the other.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
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
/// `diff` each changed path's status and line counts, `contracts` the
/// configured patterns. Pure, so a test builds it without git.
pub fn document(
    range: &Range,
    paths: &[String],
    tree: &Tree,
    diff: &BTreeMap<String, (&'static str, u64, u64)>,
    contracts: &[String],
) -> serde_json::Value {
    let is_contract = |path: &str| contracts.iter().any(|c| glob(c, path));
    let is_node = |path: &str| language_of(path).is_some() || is_contract(path);
    let mut scans: BTreeMap<String, Scan> = BTreeMap::new();
    for language in LANGUAGES {
        scans.extend(language.scan(tree));
    }
    // The tip's files, then what the phase deleted.
    let mut names: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    names.extend(diff.keys().map(String::as_str));
    let nodes: Vec<Node> = names
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
            }
        })
        .collect();
    let known: BTreeSet<&str> = nodes.iter().map(|n| n.path.as_str()).collect();
    let mut edges = BTreeSet::new();
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
    let changed: Vec<&Node> = nodes.iter().filter(|n| n.status != "unchanged").collect();
    json!({
        "phase": range.phase.map(|p| p.to_string()),
        "base": range.base,
        "tip": range.tip,
        "tip_ref": range.tip_ref,
        "totals": {
            "files": nodes.len(),
            "changed": changed.len(),
            "added": changed.iter().map(|n| n.added).sum::<u64>(),
            "removed": changed.iter().map(|n| n.removed).sum::<u64>(),
            "outside": nodes.iter().filter(|n| n.contract || !n.external.is_empty()).count(),
        },
        "nodes": nodes,
        "edges": edges,
    })
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

/// The graph document for `range`, read from git.
pub fn build(p: &Project, range: &Range) -> Result<serde_json::Value, CliError> {
    let dir = &p.work;
    let contracts = &p.config.map.contracts;
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
    Ok(document(range, &paths, &tree, &diff, contracts))
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

/// `ris map`: the document, and in text its totals and the changed
/// files, heaviest first. With `html`, also writes the page there.
pub fn output(doc: serde_json::Value, html: Option<&Path>) -> Result<Output, CliError> {
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
    if doc["base"].is_null() {
        text.push_str(
            "where the work left the main line is not known, so no file carries a diff\n",
        );
    }
    let mut changed: Vec<&serde_json::Value> = doc["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["status"] != "unchanged")
        .collect();
    changed.sort_by(|a, b| {
        b["weight"]
            .as_f64()
            .partial_cmp(&a["weight"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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
            "  {:>6} {:<6} {}{}{}\n",
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
    }
    match html {
        Some(path) => {
            std::fs::write(path, page(&doc))?;
            text.push_str(&format!(
                "wrote {}\n  next: open it in a browser; it needs no network",
                path.display()
            ));
        }
        None => text.push_str("  next: `ris map --html map.html` draws it as a page"),
    }
    Ok(Output::new(text, doc))
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

/// Path prefixes that mean a Rust file touches the outside.
const RUST_OUTSIDE: &[(&[&str], &str)] = &[
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
mod tests {
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
        let doc = json!({ "nodes": [{ "path": "a</script><b>.rs" }], "edges": [] });
        let html = page(&doc);
        assert!(!html.contains("/*D3*/") && !html.contains("/*DATA*/"));
        assert!(!html.contains("a</script>"), "the document is escaped");
        let open = "<script id=\"map-data\" type=\"application/json\">";
        let start = html.find(open).expect("the data element") + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        let back: serde_json::Value = serde_json::from_str(&html[start..end]).unwrap();
        assert_eq!(back, doc);
    }
}
