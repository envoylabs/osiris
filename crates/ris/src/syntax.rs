//! Functions, and the names code uses, read from a syntax tree: the
//! map's view inside a file, for Rust, TypeScript, Go and Clojure.
//!
//! Each language is parsed with tree-sitter. A file gives its functions
//! (a name and a line range) and its references: each qualified name
//! the code uses, at the line it is used, with the file's imports and
//! aliases resolved, so `fs::read` after `use std::fs;` is
//! `std::fs::read`, and `io/file` after `[clojure.java.io :as io]` is
//! `clojure.java.io/file`. Whether a reference touches the outside is
//! decided by patterns ([`default_effects`], extended per project), so
//! the parse itself knows nothing of effects.
//!
//! What is read is what the text says, not what the program does: a
//! method called on a value (`conn.execute(..)`) names no module and is
//! not a reference, since which type it belongs to takes a type checker.

use std::collections::BTreeMap;

use tree_sitter::{Node, Parser};

/// One function: its name as a reader finds it (a method with its type
/// in front: `Store::save`, `Store.Save`), and its first and last line,
/// counting from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub start: u64,
    pub end: u64,
}

/// A qualified name the code uses, and the line it is on.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    pub line: u64,
    pub name: String,
}

/// A call, as far as the text says what it calls: the callee's own
/// name (`load`, or with its type in front where the call names one:
/// `Store::new`), and the module the file's imports say it comes from
/// (`crate::store`, `./store`, a Go import path, a Clojure namespace),
/// or `None` when they say nothing: a name of the file's own.
///
/// A method called on a value is a call only where the text says the
/// value's type: the type the method is written in (`self`, `this`, a
/// Go method's receiver), or a parameter or a `let` with its type. It
/// is then named with that type in front, as a call that names the type
/// is. A method on any other value is not a call here: its bare name
/// could mean any type's method of that name, in the repository or not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Call {
    pub line: u64,
    pub name: String,
    pub module: Option<String>,
}

/// What a file's parse gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// In the order they start.
    pub functions: Vec<Function>,
    /// By line, then name, each once.
    pub references: Vec<Reference>,
    /// By line, each once.
    pub calls: Vec<Call>,
    /// The namespace a Clojure file declares in its `ns` form.
    pub namespace: Option<String>,
    /// What the file imports, as it names it: a TypeScript module
    /// specifier (`./store`, `fs`), a Go import path, a Clojure
    /// namespace it requires. Sorted, each once.
    pub imports: Vec<String>,
}

/// The separator between a type and its method in `language`'s names.
pub fn method_separator(language: &str) -> &'static str {
    if language == "rust" { "::" } else { "." }
}

/// The languages read inside their files; any other stays at file
/// level.
pub const SUPPORTED: &[&str] = &["rust", "typescript", "go", "clojure"];

/// The kinds of effect a pattern may name.
pub const KINDS: &[&str] = &["database", "filesystem", "network", "process"];

/// `file`'s functions and references, or `None` when its language is
/// not read inside files. `path` tells TSX from TypeScript.
pub fn parse(language: &str, path: &str, text: &str) -> Option<Parsed> {
    let grammar: tree_sitter::Language = match language {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "typescript" if path.ends_with(".tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "clojure" => tree_sitter_clojure_orchard::LANGUAGE.into(),
        _ => return None,
    };
    let mut parser = Parser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(text, None)?;
    let src = text.as_bytes();
    let root = tree.root_node();
    let mut out = Parsed::default();
    match language {
        "rust" => rust(root, src, &mut out),
        "typescript" => typescript(root, src, &mut out),
        "go" => go(root, src, &mut out),
        _ => clojure(root, src, &mut out),
    }
    out.functions
        .sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
    out.references.sort();
    out.references.dedup();
    out.calls.sort();
    out.calls.dedup();
    out.imports.sort();
    out.imports.dedup();
    Some(out)
}

/// Whether a name starts as a type does: with a capital letter.
fn is_type(segment: &str) -> bool {
    segment.chars().next().is_some_and(char::is_uppercase)
}

/// Whether `name` is `pattern` or lies under it: the pattern followed by
/// a separator (`::`, `.` or `/`), so `std::fs` covers `std::fs::read`
/// and not `std::fsx`.
pub fn matches(pattern: &str, name: &str) -> bool {
    name == pattern
        || name
            .strip_prefix(pattern)
            .is_some_and(|rest| rest.starts_with([':', '.', '/']))
}

/// The patterns shipped for `language`, each with the kind of effect it
/// means. A project adds its own in `[map.effects.<language>]`.
pub fn default_effects(language: &str) -> Vec<(String, &'static str)> {
    let listed: &[(&str, &str)] = match language {
        "typescript" => TYPESCRIPT_EFFECTS,
        "go" => GO_EFFECTS,
        "clojure" => CLOJURE_EFFECTS,
        "rust" => {
            return crate::map::RUST_OUTSIDE
                .iter()
                .map(|(path, kind)| (path.join("::"), *kind))
                .collect();
        }
        _ => &[],
    };
    listed.iter().map(|(p, k)| ((*p).to_owned(), *k)).collect()
}

/// Modules and globals: a reference is the module, then `.` and the
/// member, with a `node:` prefix dropped.
const TYPESCRIPT_EFFECTS: &[(&str, &str)] = &[
    ("fs", "filesystem"),
    ("fs-extra", "filesystem"),
    ("child_process", "process"),
    ("execa", "process"),
    ("http", "network"),
    ("https", "network"),
    ("http2", "network"),
    ("net", "network"),
    ("dgram", "network"),
    ("tls", "network"),
    ("fetch", "network"),
    ("XMLHttpRequest", "network"),
    ("WebSocket", "network"),
    ("axios", "network"),
    ("node-fetch", "network"),
    ("ws", "network"),
    ("pg", "database"),
    ("mysql", "database"),
    ("mysql2", "database"),
    ("sqlite3", "database"),
    ("better-sqlite3", "database"),
    ("mongodb", "database"),
    ("mongoose", "database"),
    ("redis", "database"),
    ("ioredis", "database"),
    ("@prisma/client", "database"),
    ("knex", "database"),
];

/// Packages by import path, then `.` and the member.
const GO_EFFECTS: &[(&str, &str)] = &[
    ("os.Open", "filesystem"),
    ("os.OpenFile", "filesystem"),
    ("os.Create", "filesystem"),
    ("os.CreateTemp", "filesystem"),
    ("os.ReadFile", "filesystem"),
    ("os.WriteFile", "filesystem"),
    ("os.ReadDir", "filesystem"),
    ("os.Remove", "filesystem"),
    ("os.RemoveAll", "filesystem"),
    ("os.Rename", "filesystem"),
    ("os.Mkdir", "filesystem"),
    ("os.MkdirAll", "filesystem"),
    ("os.MkdirTemp", "filesystem"),
    ("os.Truncate", "filesystem"),
    ("os.Chmod", "filesystem"),
    ("io/ioutil", "filesystem"),
    ("path/filepath.Walk", "filesystem"),
    ("path/filepath.WalkDir", "filesystem"),
    ("os/exec", "process"),
    ("os.StartProcess", "process"),
    ("syscall.Exec", "process"),
    ("net.Dial", "network"),
    ("net.DialTCP", "network"),
    ("net.DialUDP", "network"),
    ("net.DialTimeout", "network"),
    ("net.Listen", "network"),
    ("net.ListenTCP", "network"),
    ("net.ListenUDP", "network"),
    ("net.ListenPacket", "network"),
    ("net/http", "network"),
    ("net/rpc", "network"),
    ("net/smtp", "network"),
    ("google.golang.org/grpc", "network"),
    ("database/sql", "database"),
    ("github.com/jackc/pgx", "database"),
    ("github.com/lib/pq", "database"),
    ("github.com/mattn/go-sqlite3", "database"),
    ("gorm.io/gorm", "database"),
    ("go.mongodb.org/mongo-driver", "database"),
    ("github.com/redis/go-redis", "database"),
    ("github.com/go-redis/redis", "database"),
];

/// Namespaces, then `/` and the name; an unqualified call is
/// `clojure.core`'s.
const CLOJURE_EFFECTS: &[(&str, &str)] = &[
    ("clojure.core/slurp", "filesystem"),
    ("clojure.core/spit", "filesystem"),
    ("clojure.core/file-seq", "filesystem"),
    ("clojure.java.io", "filesystem"),
    ("babashka.fs", "filesystem"),
    ("me.raynes.fs", "filesystem"),
    ("clojure.java.shell", "process"),
    ("babashka.process", "process"),
    ("clj-http.client", "network"),
    ("org.httpkit.client", "network"),
    ("org.httpkit.server", "network"),
    ("aleph.http", "network"),
    ("hato.client", "network"),
    ("ring.adapter.jetty", "network"),
    ("next.jdbc", "database"),
    ("clojure.java.jdbc", "database"),
    ("datomic.api", "database"),
    ("datomic.client.api", "database"),
    ("monger.core", "database"),
    ("monger.collection", "database"),
    ("taoensso.carmine", "database"),
];

// ---- reading a tree ----

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    n.utf8_text(src).unwrap_or("")
}

fn first_line(n: Node) -> u64 {
    n.start_position().row as u64 + 1
}

fn last_line(n: Node) -> u64 {
    n.end_position().row as u64 + 1
}

/// Visit `node` and, while `f` says so, what it holds, in order.
fn walk<'t>(node: Node<'t>, f: &mut dyn FnMut(Node<'t>) -> bool) {
    if !f(node) {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, f);
    }
}

/// The nearest enclosing node of one of `kinds`, if any.
fn enclosing<'t>(node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    let mut at = node.parent();
    while let Some(n) = at {
        if kinds.contains(&n.kind()) {
            return Some(n);
        }
        at = n.parent();
    }
    None
}

/// A type as a name: no generics, no pointer or reference marks.
fn bare_type(t: &str) -> String {
    t.split('<')
        .next()
        .unwrap_or(t)
        .split('[')
        .next()
        .unwrap_or(t)
        .trim_start_matches(['*', '&'])
        .trim()
        .to_owned()
}

/// `name` with its first segment replaced by what an alias says it
/// stands for, the segments joined by `sep`.
fn resolve(name: &str, sep: &str, aliases: &BTreeMap<String, String>) -> String {
    let (head, rest) = match name.split_once(sep) {
        Some((h, r)) => (h, Some(r)),
        None => (name, None),
    };
    match (aliases.get(head), rest) {
        (Some(full), Some(rest)) => format!("{full}{sep}{rest}"),
        (Some(full), None) => full.clone(),
        (None, _) => name.to_owned(),
    }
}

// ---- Rust ----

fn rust(root: Node, src: &[u8], out: &mut Parsed) {
    let mut aliases = BTreeMap::new();
    walk(root, &mut |n| {
        if n.kind() == "use_declaration" {
            if let Some(arg) = n.child_by_field_name("argument") {
                rust_use(arg, "", src, &mut aliases);
            }
            return false;
        }
        true
    });
    walk(root, &mut |n| {
        match n.kind() {
            "use_declaration" | "mod_item" if n.child_by_field_name("body").is_none() => {
                return false;
            }
            "function_item" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let owner = rust_owner(n);
                    let name = text(name, src);
                    out.functions.push(Function {
                        name: match owner {
                            Some(o) => format!("{}::{name}", bare_type(text(o, src))),
                            None => name.to_owned(),
                        },
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            "call_expression" => {
                if let Some(mut callee) = n.child_by_field_name("function") {
                    if callee.kind() == "generic_function"
                        && let Some(inner) = callee.child_by_field_name("function")
                    {
                        callee = inner;
                    }
                    if let Some(call) = rust_call(callee, src, &aliases) {
                        out.calls.push(call);
                    }
                }
            }
            "scoped_identifier" | "scoped_type_identifier" => {
                // The whole path once; the shorter paths inside it are
                // not visited.
                let name: String = text(n, src).split_whitespace().collect();
                out.references.push(Reference {
                    line: first_line(n),
                    name: resolve(&name, "::", &aliases),
                });
                return false;
            }
            "token_tree" => {
                // A macro's arguments are tokens, not expressions: a
                // path in them is identifiers joined by `::` tokens.
                let mut cursor = n.walk();
                let tokens: Vec<Node> = n.children(&mut cursor).collect();
                let mut i = 0;
                while i < tokens.len() {
                    if tokens[i].kind() != "identifier" {
                        i += 1;
                        continue;
                    }
                    let mut path = vec![text(tokens[i], src)];
                    let mut j = i + 1;
                    while j + 1 < tokens.len()
                        && tokens[j].kind() == "::"
                        && tokens[j + 1].kind() == "identifier"
                    {
                        path.push(text(tokens[j + 1], src));
                        j += 2;
                    }
                    if path.len() > 1 {
                        out.references.push(Reference {
                            line: first_line(tokens[i]),
                            name: resolve(&path.join("::"), "::", &aliases),
                        });
                    }
                    i = j;
                }
            }
            "identifier" => {
                // A function called by a name a `use` line brought in.
                if n.parent().is_some_and(|p| {
                    p.kind() == "call_expression" && p.child_by_field_name("function") == Some(n)
                }) && let Some(full) = aliases.get(text(n, src))
                {
                    out.references.push(Reference {
                        line: first_line(n),
                        name: full.clone(),
                    });
                }
            }
            _ => {}
        }
        true
    });
}

/// What a Rust call's callee says it calls.
fn rust_call(callee: Node, src: &[u8], aliases: &BTreeMap<String, String>) -> Option<Call> {
    let line = first_line(callee);
    match callee.kind() {
        // `load(..)`, perhaps brought in by a `use` line.
        "identifier" => {
            let name = text(callee, src);
            let module = aliases
                .get(name)
                .and_then(|full| full.rsplit_once("::"))
                .map(|(m, _)| m.to_owned());
            Some(Call {
                line,
                name: name.to_owned(),
                module,
            })
        }
        // `x.save(..)`: a method on a value, where the text says the
        // value's type.
        "field_expression" => {
            let ty = rust_receiver(callee.child_by_field_name("value")?, src)?;
            let method = text(callee.child_by_field_name("field")?, src);
            // The type's module, when a `use` line brought the type in.
            let module = aliases
                .get(&ty)
                .and_then(|full| full.rsplit_once("::"))
                .map(|(m, _)| m.to_owned());
            Some(Call {
                line,
                name: format!("{ty}::{method}"),
                module,
            })
        }
        // `crate::store::load(..)`, `Store::new(..)`.
        "scoped_identifier" => {
            let path: String = text(callee, src).split_whitespace().collect();
            let mut full = resolve(&path, "::", aliases);
            // `Self` is the type of the `impl` it is written in.
            if let Some(rest) = full.strip_prefix("Self::")
                && let Some(ty) =
                    enclosing(callee, &["impl_item"]).and_then(|i| i.child_by_field_name("type"))
            {
                full = format!("{}::{rest}", bare_type(text(ty, src)));
            }
            let segments: Vec<&str> = full.split("::").collect();
            let (name, module) = match segments.as_slice() {
                [.., ty, f] if is_type(ty) => {
                    (format!("{ty}::{f}"), &segments[..segments.len() - 2])
                }
                [.., f] => ((*f).to_owned(), &segments[..segments.len() - 1]),
                [] => return None,
            };
            Some(Call {
                line,
                name,
                module: (!module.is_empty()).then(|| module.join("::")),
            })
        }
        _ => None,
    }
}

/// The type an `impl` or a trait gives the functions written in it.
fn rust_owner<'t>(n: Node<'t>) -> Option<Node<'t>> {
    enclosing(n, &["impl_item", "trait_item"]).and_then(|o| {
        o.child_by_field_name(if o.kind() == "impl_item" {
            "type"
        } else {
            "name"
        })
    })
}

/// A written type as the name its methods carry: `&mut Store` and
/// `store::Store` are `Store`, `Self` is the type of the `impl` it is
/// written in. `None` for a type with no one name (a tuple, a closure,
/// `impl Trait`).
fn rust_type(t: Node, src: &[u8]) -> Option<String> {
    match t.kind() {
        "type_identifier" if text(t, src) == "Self" => {
            rust_owner(t).map(|o| bare_type(text(o, src)))
        }
        "type_identifier" => Some(text(t, src).to_owned()),
        "reference_type" | "generic_type" => rust_type(t.child_by_field_name("type")?, src),
        "scoped_type_identifier" => Some(text(t.child_by_field_name("name")?, src).to_owned()),
        _ => None,
    }
}

/// The name a pattern binds, when it binds one: `x` and `mut x`.
fn rust_binding<'a>(pattern: Node, src: &'a [u8]) -> Option<&'a str> {
    match pattern.kind() {
        "identifier" => Some(text(pattern, src)),
        "mut_pattern" => pattern
            .named_child(0)
            .filter(|c| c.kind() == "identifier")
            .map(|c| text(c, src)),
        _ => None,
    }
}

/// The type a `let` gives its name by what it is set to, where the text
/// says it: a struct written out (`Store { .. }`), or `Store::new(..)`
/// and `Store::default()`, which return their own type by convention.
fn rust_value_type(value: Node, src: &[u8]) -> Option<String> {
    match value.kind() {
        "struct_expression" => rust_type(value.child_by_field_name("name")?, src),
        "call_expression" => {
            let f = value.child_by_field_name("function")?;
            if f.kind() != "scoped_identifier" {
                return None;
            }
            let made = text(f.child_by_field_name("name")?, src);
            let ty = f.child_by_field_name("path")?;
            if !matches!(made, "new" | "default") {
                return None;
            }
            match ty.kind() {
                "identifier" if text(ty, src) == "Self" => {
                    rust_owner(ty).map(|o| bare_type(text(o, src)))
                }
                "identifier" if is_type(text(ty, src)) => Some(text(ty, src).to_owned()),
                "scoped_identifier" => ty
                    .child_by_field_name("name")
                    .map(|n| text(n, src).to_owned())
                    .filter(|n| is_type(n)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The type of the value a method is called on, where the text of the
/// function it is in says it: `self`, a parameter with its type, or a
/// name a `let` above gave a type ([`rust_value_type`], or one written
/// after the name). `None` for anything else (a field, the result of a
/// call, a name bound by a pattern), whose type takes a type checker.
fn rust_receiver(value: Node, src: &[u8]) -> Option<String> {
    if value.kind() == "self" {
        return rust_owner(value).map(|o| bare_type(text(o, src)));
    }
    if value.kind() != "identifier" {
        return None;
    }
    let name = text(value, src);
    // The nearest closure or function that takes the name says its type;
    // a closure's parameter with no type written hides any outside it.
    let mut at = value.parent();
    let mut function = None;
    while let Some(n) = at {
        if matches!(n.kind(), "closure_expression" | "function_item")
            && let Some(parameters) = n.child_by_field_name("parameters")
        {
            let mut cursor = parameters.walk();
            for p in parameters.named_children(&mut cursor) {
                let pattern = if p.kind() == "parameter" {
                    p.child_by_field_name("pattern")
                } else {
                    Some(p)
                };
                if pattern.and_then(|b| rust_binding(b, src)) == Some(name) {
                    return rust_type(p.child_by_field_name("type")?, src);
                }
            }
        }
        if n.kind() == "function_item" {
            function = Some(n);
            break;
        }
        at = n.parent();
    }
    // The last `let` of the name above the call, in the same function.
    let mut found = None;
    walk(function?, &mut |n| {
        if n.start_byte() >= value.start_byte() {
            return false;
        }
        if n.kind() == "let_declaration"
            && n.end_byte() <= value.start_byte()
            && n.child_by_field_name("pattern")
                .and_then(|b| rust_binding(b, src))
                == Some(name)
        {
            found = Some(match n.child_by_field_name("type") {
                Some(t) => rust_type(t, src),
                None => n
                    .child_by_field_name("value")
                    .and_then(|v| rust_value_type(v, src)),
            });
        }
        true
    });
    found.flatten()
}

fn join_rust(prefix: &str, s: &str) -> String {
    if prefix.is_empty() {
        s.to_owned()
    } else {
        format!("{prefix}::{s}")
    }
}

/// A `use` tree's names: the last segment of each path (or its `as`
/// name) to the whole path.
fn rust_use(n: Node, prefix: &str, src: &[u8], aliases: &mut BTreeMap<String, String>) {
    match n.kind() {
        "scoped_identifier" | "identifier" | "crate" | "super" => {
            let full = join_rust(prefix, &text(n, src).split_whitespace().collect::<String>());
            let last = full.rsplit("::").next().unwrap_or(&full).to_owned();
            aliases.insert(last, full);
        }
        // `self` in a list names the list's own path.
        "self" if !prefix.is_empty() => {
            let last = prefix.rsplit("::").next().unwrap_or(prefix).to_owned();
            aliases.insert(last, prefix.to_owned());
        }
        "use_as_clause" => {
            if let (Some(path), Some(alias)) = (
                n.child_by_field_name("path"),
                n.child_by_field_name("alias"),
            ) {
                let path: String = text(path, src).split_whitespace().collect();
                let full = if path == "self" {
                    prefix.to_owned()
                } else {
                    join_rust(prefix, &path)
                };
                aliases.insert(text(alias, src).to_owned(), full);
            }
        }
        "scoped_use_list" => {
            let inner = match n.child_by_field_name("path") {
                Some(p) => join_rust(prefix, &text(p, src).split_whitespace().collect::<String>()),
                None => prefix.to_owned(),
            };
            if let Some(list) = n.child_by_field_name("list") {
                rust_use(list, &inner, src, aliases);
            }
        }
        "use_list" => {
            let mut cursor = n.walk();
            for child in n.named_children(&mut cursor) {
                rust_use(child, prefix, src, aliases);
            }
        }
        _ => {}
    }
}

// ---- TypeScript ----

/// A module as a reference names it: no quotes, no `node:` prefix.
fn module_name(literal: &str) -> String {
    let m = literal.trim_matches(['"', '\'', '`']);
    m.strip_prefix("node:").unwrap_or(m).to_owned()
}

/// The identifiers of a member chain (`fs.promises.readFile`), or
/// `None` when it holds anything else (a call, `this`, an index).
fn member_chain(n: Node, src: &[u8]) -> Option<Vec<String>> {
    match n.kind() {
        "identifier" | "property_identifier" => Some(vec![text(n, src).to_owned()]),
        "member_expression" => {
            let mut chain = member_chain(n.child_by_field_name("object")?, src)?;
            chain.push(text(n.child_by_field_name("property")?, src).to_owned());
            Some(chain)
        }
        _ => None,
    }
}

/// The type written for the parameter `name` names, in the nearest
/// function that takes one of that name; `None` when none does, or its
/// type is not one name.
fn typescript_parameter(name: Node, src: &[u8]) -> Option<String> {
    let wanted = text(name, src);
    let mut at = name.parent();
    while let Some(n) = at {
        if let Some(parameters) = n.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for p in parameters.named_children(&mut cursor) {
                if p.child_by_field_name("pattern").map(|b| text(b, src)) != Some(wanted) {
                    continue;
                }
                let ty = p.child_by_field_name("type")?.named_child(0)?;
                return (ty.kind() == "type_identifier").then(|| text(ty, src).to_owned());
            }
        }
        at = n.parent();
    }
    None
}

fn typescript(root: Node, src: &[u8], out: &mut Parsed) {
    // Each imported name: the module, and the member it names (none for
    // a default or namespace import, or a `require`).
    let mut imports: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    walk(root, &mut |n| {
        match n.kind() {
            "import_statement" => {
                let Some(source) = n.child_by_field_name("source") else {
                    return false;
                };
                let module = module_name(text(source, src));
                out.imports.push(module.clone());
                walk(n, &mut |c| {
                    match c.kind() {
                        // `import fs from "fs"`: the default export.
                        // `import * as fs from "fs"`.
                        "import_clause" | "namespace_import" => {
                            let mut cursor = c.walk();
                            for d in c.named_children(&mut cursor) {
                                if d.kind() == "identifier" {
                                    imports.insert(text(d, src).to_owned(), (module.clone(), None));
                                }
                            }
                        }
                        // `import { readFile as read } from "fs"`.
                        "import_specifier" => {
                            if let Some(name) = c.child_by_field_name("name") {
                                let local = c.child_by_field_name("alias").unwrap_or(name);
                                imports.insert(
                                    text(local, src).to_owned(),
                                    (module.clone(), Some(text(name, src).to_owned())),
                                );
                            }
                        }
                        _ => {}
                    }
                    true
                });
                false
            }
            // `const fs = require("fs")`.
            "variable_declarator" => {
                if let (Some(name), Some(value)) = (
                    n.child_by_field_name("name"),
                    n.child_by_field_name("value"),
                ) && name.kind() == "identifier"
                    && value.kind() == "call_expression"
                    && value
                        .child_by_field_name("function")
                        .is_some_and(|f| text(f, src) == "require")
                    && let Some(args) = value.child_by_field_name("arguments")
                    && let Some(first) = args.named_child(0)
                    && first.kind() == "string"
                {
                    imports.insert(
                        text(name, src).to_owned(),
                        (module_name(text(first, src)), None),
                    );
                }
                true
            }
            // `export { save } from "./store"`.
            "export_statement" => {
                if let Some(source) = n.child_by_field_name("source") {
                    out.imports.push(module_name(text(source, src)));
                }
                true
            }
            // `require("./store")` wherever it is, and `import("./store")`.
            "call_expression" => {
                if let Some(f) = n.child_by_field_name("function")
                    && matches!(text(f, src), "require" | "import")
                    && let Some(args) = n.child_by_field_name("arguments")
                    && let Some(first) = args.named_child(0)
                    && first.kind() == "string"
                {
                    out.imports.push(module_name(text(first, src)));
                }
                true
            }
            _ => true,
        }
    });
    // What each imported name stands for, as a reference names it.
    let aliases: BTreeMap<String, String> = imports
        .iter()
        .map(|(local, (module, member))| {
            let full = match member {
                Some(m) => format!("{module}.{m}"),
                None => module.clone(),
            };
            (local.clone(), full)
        })
        .collect();
    walk(root, &mut |n| {
        match n.kind() {
            "import_statement" => return false,
            "function_declaration" | "generator_function_declaration" => {
                let name = n
                    .child_by_field_name("name")
                    .map_or("default", |m| text(m, src));
                out.functions.push(Function {
                    name: name.to_owned(),
                    start: first_line(n),
                    end: last_line(n),
                });
            }
            "method_definition" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let class = enclosing(
                        n,
                        &["class_declaration", "abstract_class_declaration", "class"],
                    )
                    .and_then(|c| c.child_by_field_name("name"))
                    .map(|c| text(c, src));
                    let name = text(name, src);
                    out.functions.push(Function {
                        name: class.map_or(name.to_owned(), |c| format!("{c}.{name}")),
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            "variable_declarator" => {
                if let (Some(name), Some(value)) = (
                    n.child_by_field_name("name"),
                    n.child_by_field_name("value"),
                ) && matches!(value.kind(), "arrow_function" | "function_expression")
                {
                    out.functions.push(Function {
                        name: text(name, src).to_owned(),
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            "call_expression" => {
                let callee = n.child_by_field_name("function");
                let call = match callee.map(|c| (c, c.kind())) {
                    // `load(..)`: the file's own, or imported.
                    Some((c, "identifier")) => {
                        let local = text(c, src);
                        Some(match imports.get(local) {
                            Some((module, member)) => (
                                member.clone().unwrap_or_else(|| local.to_owned()),
                                Some(module.clone()),
                            ),
                            None => (local.to_owned(), None),
                        })
                    }
                    // `store.save(..)` on an imported name: that module's
                    // function. `this.save(..)`, and a method on a
                    // parameter with its type: that type's method. A
                    // method on any other value is not a call here.
                    Some((c, "member_expression")) => {
                        let method = c.child_by_field_name("property").map(|p| text(p, src));
                        let object = c.child_by_field_name("object");
                        match (method, object) {
                            (Some(method), Some(o)) if o.kind() == "this" => enclosing(
                                o,
                                &["class_declaration", "abstract_class_declaration", "class"],
                            )
                            .and_then(|class| class.child_by_field_name("name"))
                            .map(|class| (format!("{}.{method}", text(class, src)), None)),
                            (Some(method), Some(o)) if o.kind() == "identifier" => {
                                match imports.get(text(o, src)) {
                                    Some((module, _)) => {
                                        Some((method.to_owned(), Some(module.clone())))
                                    }
                                    None => typescript_parameter(o, src).map(|ty| {
                                        let module = imports.get(&ty).map(|(m, _)| m.clone());
                                        (format!("{ty}.{method}"), module)
                                    }),
                                }
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((name, module)) = call {
                    out.calls.push(Call {
                        line: first_line(n),
                        name,
                        module,
                    });
                }
            }
            "member_expression" => {
                // The whole chain once, resolved through an import.
                if n.parent().is_some_and(|p| {
                    p.kind() == "member_expression" && p.child_by_field_name("object") == Some(n)
                }) {
                    return false;
                }
                if let Some(chain) = member_chain(n, src)
                    && let Some(module) = aliases.get(&chain[0])
                {
                    out.references.push(Reference {
                        line: first_line(n),
                        name: format!("{module}.{}", chain[1..].join(".")),
                    });
                }
                return true;
            }
            "identifier" => {
                // Called or constructed: an imported name, or a global
                // such as `fetch`.
                let called = n.parent().is_some_and(|p| {
                    matches!(p.kind(), "call_expression" | "new_expression")
                        && (p.child_by_field_name("function") == Some(n)
                            || p.child_by_field_name("constructor") == Some(n))
                });
                if called {
                    let name = text(n, src);
                    out.references.push(Reference {
                        line: first_line(n),
                        name: aliases
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.to_owned()),
                    });
                }
            }
            _ => {}
        }
        true
    });
}

// ---- Go ----

/// The name a Go import is used by: the one given, or the path's last
/// segment that is not a major version (`pgx/v5` is `pgx`).
fn go_package(path: &str) -> &str {
    let mut segments = path.rsplit('/');
    let last = segments.next().unwrap_or(path);
    let is_version =
        last.len() > 1 && last.starts_with('v') && last[1..].chars().all(|c| c.is_ascii_digit());
    if is_version {
        segments.next().unwrap_or(last)
    } else {
        last
    }
}

/// The type written for the receiver or parameter `name` names, in the
/// nearest function that takes one of that name, as its methods carry
/// it (`*Store` is `Store`; a type of another package keeps the
/// package: `store.Store`). `None` when no function around it does.
fn go_parameter(name: Node, src: &[u8]) -> Option<String> {
    let wanted = text(name, src);
    let mut at = name.parent();
    while let Some(n) = at {
        for field in ["receiver", "parameters"] {
            let Some(list) = n.child_by_field_name(field) else {
                continue;
            };
            let mut cursor = list.walk();
            for p in list.named_children(&mut cursor) {
                let mut names = p.walk();
                if p.children_by_field_name("name", &mut names)
                    .any(|m| text(m, src) == wanted)
                {
                    return p
                        .child_by_field_name("type")
                        .map(|t| bare_type(text(t, src)));
                }
            }
        }
        at = n.parent();
    }
    None
}

fn go(root: Node, src: &[u8], out: &mut Parsed) {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    walk(root, &mut |n| {
        if n.kind() == "import_spec" {
            if let Some(path) = n.child_by_field_name("path") {
                let path = text(path, src).trim_matches(['"', '`']).to_owned();
                out.imports.push(path.clone());
                let local = n
                    .child_by_field_name("name")
                    .map(|m| text(m, src).to_owned())
                    .unwrap_or_else(|| go_package(&path).to_owned());
                if local != "_" && local != "." {
                    aliases.insert(local, path);
                }
            }
            return false;
        }
        true
    });
    walk(root, &mut |n| {
        match n.kind() {
            "import_declaration" => return false,
            "function_declaration" => {
                if let Some(name) = n.child_by_field_name("name") {
                    out.functions.push(Function {
                        name: text(name, src).to_owned(),
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            "method_declaration" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let receiver = n
                        .child_by_field_name("receiver")
                        .and_then(|r| r.named_child(0))
                        .and_then(|p| p.child_by_field_name("type"))
                        .map(|t| bare_type(text(t, src)));
                    let name = text(name, src);
                    out.functions.push(Function {
                        name: receiver.map_or(name.to_owned(), |r| format!("{r}.{name}")),
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            "call_expression" => {
                let call = match n.child_by_field_name("function") {
                    // `load(..)`: the package's own.
                    Some(f) if f.kind() == "identifier" => Some((text(f, src).to_owned(), None)),
                    // `store.Load(..)` through an import: that package's
                    // function. `s.Save(..)` on a method's receiver or
                    // a parameter: its type's method. A method on any
                    // other value is not a call here.
                    Some(f) if f.kind() == "selector_expression" => {
                        let field = f.child_by_field_name("field").map(|x| text(x, src));
                        let operand = f
                            .child_by_field_name("operand")
                            .filter(|o| o.kind() == "identifier");
                        match (field, operand) {
                            (Some(field), Some(o)) => match aliases.get(text(o, src)) {
                                Some(path) => Some((field.to_owned(), Some(path.clone()))),
                                None => go_parameter(o, src).map(|ty| match ty.split_once('.') {
                                    // `store.Store`: a type of an import.
                                    Some((package, ty)) => {
                                        (format!("{ty}.{field}"), aliases.get(package).cloned())
                                    }
                                    None => (format!("{ty}.{field}"), None),
                                }),
                            },
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((name, module)) = call {
                    out.calls.push(Call {
                        line: first_line(n),
                        name,
                        module,
                    });
                }
            }
            // `pkg.Name` as a value, and as a type.
            "selector_expression" | "qualified_type" => {
                let (operand, field) = if n.kind() == "qualified_type" {
                    ("package", "name")
                } else {
                    ("operand", "field")
                };
                if let (Some(operand), Some(field)) =
                    (n.child_by_field_name(operand), n.child_by_field_name(field))
                    && matches!(operand.kind(), "identifier" | "package_identifier")
                    && let Some(path) = aliases.get(text(operand, src))
                {
                    out.references.push(Reference {
                        line: first_line(n),
                        name: format!("{path}.{}", text(field, src)),
                    });
                }
            }
            _ => {}
        }
        true
    });
}

// ---- Clojure ----

/// The forms of a list, in order, comments and discarded forms left out.
fn values<'t>(list: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = list.walk();
    list.children_by_field_name("value", &mut cursor).collect()
}

/// A symbol as `ns/name` or `name`, without any metadata written on it.
fn symbol(n: Node, src: &[u8]) -> Option<String> {
    if n.kind() != "sym_lit" {
        return None;
    }
    let name = text(n.child_by_field_name("name")?, src);
    Some(match n.child_by_field_name("namespace") {
        Some(ns) => format!("{}/{name}", text(ns, src)),
        None => name.to_owned(),
    })
}

/// The aliases and referred names of an `ns` form's `:require` clauses,
/// and every namespace its `:require` and `:use` clauses name.
fn clojure_requires(
    ns: Node,
    src: &[u8],
    aliases: &mut BTreeMap<String, String>,
    imports: &mut Vec<String>,
) {
    for clause in values(ns) {
        if clause.kind() != "list_lit" {
            continue;
        }
        let parts = values(clause);
        let Some(head @ (":require" | ":use")) = parts.first().map(|k| text(*k, src)) else {
            continue;
        };
        for spec in &parts[1..] {
            // `clojure.string`, bare.
            if let Some(lib) = symbol(*spec, src) {
                imports.push(lib);
                continue;
            }
            if spec.kind() != "vec_lit" {
                continue;
            }
            let items = values(*spec);
            let Some(lib) = items.first().and_then(|n| symbol(*n, src)) else {
                continue;
            };
            imports.push(lib.clone());
            if head == ":use" {
                continue;
            }
            let mut i = 1;
            while i + 1 < items.len() {
                match text(items[i], src) {
                    ":as" | ":as-alias" => {
                        if let Some(alias) = symbol(items[i + 1], src) {
                            aliases.insert(alias, lib.clone());
                        }
                    }
                    ":refer" if items[i + 1].kind() == "vec_lit" => {
                        for name in values(items[i + 1]) {
                            if let Some(name) = symbol(name, src) {
                                aliases.insert(name.clone(), format!("{lib}/{name}"));
                            }
                        }
                    }
                    _ => {}
                }
                i += 2;
            }
        }
    }
}

fn clojure(root: Node, src: &[u8], out: &mut Parsed) {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    walk(root, &mut |n| {
        if n.kind() == "list_lit"
            && values(n).first().and_then(|h| symbol(*h, src)).as_deref() == Some("ns")
        {
            out.namespace = values(n).get(1).and_then(|s| symbol(*s, src));
            clojure_requires(n, src, &mut aliases, &mut out.imports);
            return false;
        }
        true
    });
    walk(root, &mut |n| {
        if n.kind() != "list_lit" {
            return true;
        }
        let forms = values(n);
        let Some(head) = forms.first().and_then(|h| symbol(*h, src)) else {
            return true;
        };
        match head.as_str() {
            "ns" => return false,
            "defn" | "defn-" | "defmacro" | "defmethod" => {
                if let Some(name) = forms.get(1).and_then(|m| symbol(*m, src)) {
                    out.functions.push(Function {
                        name,
                        start: first_line(n),
                        end: last_line(n),
                    });
                }
            }
            _ => {}
        }
        // The call: `ns/f`, a referred `f`, or the file's own `f`.
        let (called, module) = match head.split_once('/') {
            Some((ns, f)) if !ns.is_empty() && !f.is_empty() => (
                f.to_owned(),
                Some(aliases.get(ns).map_or(ns, String::as_str).to_owned()),
            ),
            _ => match aliases.get(&head).and_then(|full| full.split_once('/')) {
                Some((ns, f)) => (f.to_owned(), Some(ns.to_owned())),
                None => (head.clone(), None),
            },
        };
        if !matches!(
            head.as_str(),
            "defn" | "defn-" | "defmacro" | "defmethod" | "def" | "fn" | "let" | "if" | "do"
        ) {
            out.calls.push(Call {
                line: forms.first().map_or(first_line(n), |h| first_line(*h)),
                name: called,
                module,
            });
        }
        let name = match head.split_once('/') {
            Some((ns, name)) if !ns.is_empty() && !name.is_empty() => {
                format!("{}/{name}", aliases.get(ns).map_or(ns, String::as_str))
            }
            _ => aliases
                .get(&head)
                .cloned()
                .unwrap_or_else(|| format!("clojure.core/{head}")),
        };
        out.references.push(Reference {
            line: forms.first().map_or(first_line(n), |h| first_line(*h)),
            name,
        });
        true
    });
}

#[cfg(test)]
#[path = "tests/syntax.rs"]
mod tests;
