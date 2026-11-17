use super::*;

fn names(p: &Parsed) -> Vec<(&str, u64, u64)> {
    p.functions
        .iter()
        .map(|f| (f.name.as_str(), f.start, f.end))
        .collect()
}

fn refs(p: &Parsed) -> Vec<(u64, &str)> {
    p.references
        .iter()
        .map(|r| (r.line, r.name.as_str()))
        .collect()
}

/// The kinds the shipped patterns give each line of a file.
fn effects(language: &str, p: &Parsed) -> Vec<(u64, &'static str)> {
    let patterns = default_effects(language);
    let mut out: Vec<(u64, &'static str)> = p
        .references
        .iter()
        .flat_map(|r| {
            patterns
                .iter()
                .filter(|(pat, _)| matches(pat, &r.name))
                .map(move |(_, kind)| (r.line, *kind))
        })
        .collect();
    out.dedup();
    out
}

#[test]
fn a_pattern_covers_what_lies_under_it_and_nothing_else() {
    assert!(matches("std::fs", "std::fs"));
    assert!(matches("std::fs", "std::fs::read"));
    assert!(!matches("std::fs", "std::fsx::read"));
    assert!(matches("fs", "fs.readFile"));
    assert!(matches("fs", "fs/promises.readFile"));
    assert!(!matches("fs", "fsevents.watch"));
    assert!(matches("clojure.java.io", "clojure.java.io/file"));
    assert!(!matches("net.Dial", "net.DialContext"));
}

#[test]
fn rust_functions_and_references_resolve_through_use_lines() {
    let text = "\
use std::fs;
use std::fs::File as F;
use std::process::{self, Command};
use std::fs::read_to_string;

pub struct Store;

impl Store {
    pub fn save(&self) {
        fs::write(\"x\", b\"\").unwrap();
        let _ = F::open(\"x\");
    }
}

fn run() {
    Command::new(\"ls\").status().unwrap();
    process::exit(0);
}

fn load() -> String {
    let s = format!(\"{}\", fs::read_to_string(\"y\").unwrap());
    read_to_string(\"z\").unwrap() + &s
}
";
    let p = parse("rust", "src/store.rs", text).unwrap();
    assert_eq!(
        names(&p),
        [("Store::save", 9, 12), ("run", 15, 18), ("load", 20, 23)]
    );
    let r = refs(&p);
    assert!(r.contains(&(10, "std::fs::write")), "{r:?}");
    assert!(r.contains(&(11, "std::fs::File::open")), "{r:?}");
    assert!(r.contains(&(16, "std::process::Command::new")), "{r:?}");
    assert!(r.contains(&(17, "std::process::exit")), "{r:?}");
    // Inside a macro's arguments, and called by an imported name.
    assert!(r.contains(&(21, "std::fs::read_to_string")), "{r:?}");
    assert!(r.contains(&(22, "std::fs::read_to_string")), "{r:?}");
    // The `use` lines themselves are not references.
    assert!(r.iter().all(|(line, _)| *line > 4), "{r:?}");
    // Ending the process is not running one.
    assert_eq!(
        effects("rust", &p),
        [
            (10, "filesystem"),
            (11, "filesystem"),
            (16, "process"),
            (21, "filesystem"),
            (22, "filesystem")
        ]
    );
}

#[test]
fn typescript_functions_and_references_resolve_through_imports() {
    let text = "\
import fs from \"node:fs\";
import { readFile as read } from \"fs/promises\";
import * as http from \"http\";
const cp = require(\"child_process\");

export function load(p: string) {
  return fs.readFileSync(p, \"utf8\");
}

export class Client {
  async get(url: string) {
    await fetch(url);
    return http.request(url);
  }
}

const run = async () => {
  cp.execSync(\"ls\");
  await read(\"x\");
};
";
    let p = parse("typescript", "src/client.ts", text).unwrap();
    assert_eq!(
        names(&p),
        [("load", 6, 8), ("Client.get", 11, 14), ("run", 17, 20)]
    );
    let r = refs(&p);
    assert!(r.contains(&(7, "fs.readFileSync")), "{r:?}");
    assert!(r.contains(&(12, "fetch")), "{r:?}");
    assert!(r.contains(&(13, "http.request")), "{r:?}");
    assert!(r.contains(&(18, "child_process.execSync")), "{r:?}");
    assert!(r.contains(&(19, "fs/promises.readFile")), "{r:?}");
    assert_eq!(
        effects("typescript", &p),
        [
            (7, "filesystem"),
            (12, "network"),
            (13, "network"),
            (18, "process"),
            (19, "filesystem")
        ]
    );
    // TSX parses with its own grammar.
    let tsx = parse(
        "typescript",
        "src/view.tsx",
        "export function View() {\n  return <div>{fetch(\"x\") && 1}</div>;\n}\n",
    )
    .unwrap();
    assert_eq!(names(&tsx), [("View", 1, 3)]);
    assert!(refs(&tsx).contains(&(2, "fetch")));
}

#[test]
fn go_functions_and_references_resolve_through_imports() {
    let text = "\
package store

import (
\t\"database/sql\"
\t\"net/http\"
\t\"os\"
\tpgx \"github.com/jackc/pgx/v5\"
)

type Store struct{ db *sql.DB }

func (s *Store) Save(p string) error {
\t_, err := os.ReadFile(p)
\treturn err
}

func Fetch(url string) {
\thttp.Get(url)
\tpgx.Connect(nil, url)
\tos.Getenv(\"HOME\")
}
";
    let p = parse("go", "store/store.go", text).unwrap();
    assert_eq!(names(&p), [("Store.Save", 12, 15), ("Fetch", 17, 21)]);
    let r = refs(&p);
    assert!(r.contains(&(10, "database/sql.DB")), "{r:?}");
    assert!(r.contains(&(13, "os.ReadFile")), "{r:?}");
    assert!(r.contains(&(18, "net/http.Get")), "{r:?}");
    assert!(
        r.contains(&(19, "github.com/jackc/pgx/v5.Connect")),
        "{r:?}"
    );
    // Reading the environment touches nothing outside.
    assert_eq!(
        effects("go", &p),
        [
            (10, "database"),
            (13, "filesystem"),
            (18, "network"),
            (19, "database")
        ]
    );
}

#[test]
fn clojure_functions_and_references_resolve_through_requires() {
    let text = "\
(ns app.store
  (:require [clojure.java.io :as io]
            [next.jdbc :as jdbc]
            [clojure.java.shell :refer [sh]]))

(defn ^:private save [p]
  (spit p \"x\")
  (io/file p))

(defn query [ds]
  (jdbc/execute! ds [\"select 1\"])
  (sh \"ls\")
  (str \"done\"))
";
    let p = parse("clojure", "src/app/store.clj", text).unwrap();
    assert_eq!(names(&p), [("save", 6, 8), ("query", 10, 13)]);
    let r = refs(&p);
    assert!(r.contains(&(7, "clojure.core/spit")), "{r:?}");
    assert!(r.contains(&(8, "clojure.java.io/file")), "{r:?}");
    assert!(r.contains(&(11, "next.jdbc/execute!")), "{r:?}");
    assert!(r.contains(&(12, "clojure.java.shell/sh")), "{r:?}");
    assert_eq!(
        effects("clojure", &p),
        [
            (7, "filesystem"),
            (8, "filesystem"),
            (11, "database"),
            (12, "process")
        ]
    );
}

fn calls(p: &Parsed) -> Vec<(u64, &str, Option<&str>)> {
    p.calls
        .iter()
        .map(|c| (c.line, c.name.as_str(), c.module.as_deref()))
        .collect()
}

#[test]
fn a_call_names_its_callee_and_the_module_imports_say_it_is_from() {
    let rust = parse(
        "rust",
        "src/app.rs",
        "use crate::store::{load, Store};\n\
         impl Store {\n    fn fresh() -> Self { Self::new() }\n}\n\
         fn run(s: Store) {\n    load();\n    Store::open();\n    s.save();\n    \
         crate::net::send::<u8>();\n    helper();\n}\n",
    )
    .unwrap();
    assert_eq!(
        calls(&rust),
        [
            // `Self` is the impl's own type: no import says more.
            (3, "Store::new", None),
            (6, "load", Some("crate::store")),
            (7, "Store::open", Some("crate::store")),
            // The parameter's type says whose `save`.
            (8, "Store::save", Some("crate::store")),
            (9, "send", Some("crate::net")),
            (10, "helper", None),
        ]
    );
    let ts = parse(
        "typescript",
        "src/app.ts",
        "import { load as get } from \"./store\";\nimport * as fs from \"fs\";\n\
         function run() {\n  get();\n  fs.readFileSync(\"x\");\n  this.save();\n  helper();\n}\n",
    )
    .unwrap();
    assert_eq!(
        calls(&ts),
        [
            (4, "load", Some("./store")),
            (5, "readFileSync", Some("fs")),
            // `this` outside any class names no type: not a call.
            (7, "helper", None),
        ]
    );
    let go = parse(
        "go",
        "cmd/app.go",
        "package main\n\nimport \"example.com/app/store\"\n\n\
         func run(s store.Store) {\n\tstore.Load()\n\ts.Save()\n\thelper()\n}\n",
    )
    .unwrap();
    assert_eq!(
        calls(&go),
        [
            (6, "Load", Some("example.com/app/store")),
            // The parameter's type, and the package it is imported from.
            (7, "Store.Save", Some("example.com/app/store")),
            (8, "helper", None),
        ]
    );
    let clj = parse(
        "clojure",
        "src/app/core.clj",
        "(ns app.core\n  (:require [app.store :as store :refer [load]]))\n\n\
         (defn run []\n  (store/save)\n  (load)\n  (helper))\n",
    )
    .unwrap();
    assert_eq!(clj.namespace.as_deref(), Some("app.core"));
    assert_eq!(
        calls(&clj),
        [
            (5, "save", Some("app.store")),
            (6, "load", Some("app.store")),
            (7, "helper", None),
        ]
    );
}

#[test]
fn a_method_on_a_value_is_a_call_only_where_the_text_says_its_type() {
    let rust = parse(
        "rust",
        "src/app.rs",
        "use std::process::Command;\n\
         struct App { store: Store }\n\
         impl App {\n\
         \x20   fn go(&self, other: &mut Self, cmd: Command, names: Vec<String>) {\n\
         \x20       self.step();\n\
         \x20       other.step();\n\
         \x20       cmd.output();\n\
         \x20       names.get(0);\n\
         \x20       self.store.get();\n\
         \x20       make().get();\n\
         \x20       let a = Store::new();\n\
         \x20       a.get();\n\
         \x20       let b = Store { n: 1 };\n\
         \x20       b.get();\n\
         \x20       let c: store::Store = make();\n\
         \x20       c.get();\n\
         \x20       let d = make();\n\
         \x20       d.get();\n\
         \x20       let e = make();\n\
         \x20       e.get();\n\
         \x20       let each = |s: &Store, t| { s.get(); t.get(); };\n\
         \x20   }\n\
         }\n",
    )
    .unwrap();
    assert_eq!(
        calls(&rust),
        [
            // `self` and `Self` are the impl's type.
            (5, "App::step", None),
            (6, "App::step", None),
            // A parameter's type, with the module its `use` line names:
            // another crate's, so not the repository's own `output`.
            (7, "Command::output", Some("std::process")),
            // A generic type is its own name: `Vec`, never `String`.
            (8, "Vec::get", None),
            // Line 9 is a field's method and line 10 a result's: no type
            // is written, so neither is a call.
            (10, "make", None),
            // A `let` set to `Type::new(..)` or to a struct written out.
            (11, "Store::new", None),
            (12, "Store::get", None),
            (14, "Store::get", None),
            // A `let` with its type written.
            (15, "make", None),
            (16, "Store::get", None),
            // A `let` set to a call says nothing (lines 18 and 20).
            (17, "make", None),
            (19, "make", None),
            // A closure's typed parameter; its untyped one says nothing.
            (21, "Store::get", None),
        ]
    );

    let ts = parse(
        "typescript",
        "src/app.ts",
        "import { Store } from \"./store\";\n\
         class App {\n\
         \x20 go(store: Store, other) {\n\
         \x20   this.step();\n\
         \x20   store.save();\n\
         \x20   other.save();\n\
         \x20   make().save();\n\
         \x20 }\n\
         }\n",
    )
    .unwrap();
    assert_eq!(
        calls(&ts),
        [
            (4, "App.step", None),
            (5, "Store.save", Some("./store")),
            // Lines 6 and 7: no type is written.
            (7, "make", None),
        ]
    );

    let go = parse(
        "go",
        "store/store.go",
        "package store\n\n\
         func (s *Store) Save(other Store, x, y Item) {\n\
         \ts.flush()\n\
         \tother.flush()\n\
         \ty.Name()\n\
         \tmake().flush()\n\
         \tz := load()\n\
         \tz.flush()\n\
         }\n",
    )
    .unwrap();
    assert_eq!(
        calls(&go),
        [
            // The method's receiver, and its parameters.
            (4, "Store.flush", None),
            (5, "Store.flush", None),
            (6, "Item.Name", None),
            // Lines 7 and 9: no type is written.
            (7, "make", None),
            (8, "load", None),
        ]
    );
}

#[test]
fn a_name_bound_more_than_once_says_nothing_of_its_type() {
    // Each function binds `s` as a `Store` and binds it again, or binds
    // it where the call is not: which `s` the call sees takes the
    // scopes, so no `s.get()` is `Store::get`.
    let rust = parse(
        "rust",
        "src/app.rs",
        "fn a(s: Store) {\n    let s = s.name();\n    s.get();\n}\n\
         fn b(names: Vec<Name>) {\n    let s: Store = make();\n    \
         for s in names {\n        s.get();\n    }\n}\n\
         fn c(s: Store, o: Option<Name>) {\n    if let Some(s) = o {\n        s.get();\n    }\n}\n\
         fn d(s: Store, o: Option<Name>) {\n    match o {\n        Some(s) => s.get(),\n        \
         None => {}\n    }\n}\n\
         fn e() {\n    {\n        let s: Store = make();\n    }\n    s.get();\n}\n\
         fn f(s: Store) {\n    let g = |s| s.get();\n}\n\
         fn once(s: Store) {\n    s.get();\n}\n",
    )
    .unwrap();
    let methods: Vec<(u64, &str)> = calls(&rust)
        .into_iter()
        .filter(|(_, name, _)| name.contains("::"))
        .map(|(line, name, _)| (line, name))
        .collect();
    // In `a` the parameter is still the one meant at `s.name()`, but the
    // text alone does not say so. Only `once` binds the name once.
    assert_eq!(methods, [(32, "Store::get")], "{:?}", calls(&rust));

    let go = parse(
        "go",
        "store/store.go",
        "package store\n\n\
         func (s *Store) A(names []Name) {\n\tif true {\n\t\ts := names[0]\n\t\ts.Get()\n\t}\n}\n\n\
         func (s *Store) B(names []Name) {\n\tfor _, s := range names {\n\t\ts.Get()\n\t}\n}\n\n\
         func (s *Store) Once() {\n\ts.Get()\n}\n",
    )
    .unwrap();
    let methods: Vec<(u64, &str)> = calls(&go)
        .into_iter()
        .filter(|(_, name, _)| name.contains('.'))
        .map(|(line, name, _)| (line, name))
        .collect();
    assert_eq!(methods, [(17, "Store.Get")], "{:?}", calls(&go));

    let ts = parse(
        "typescript",
        "src/app.ts",
        "function a(s: Store, names: Name[]) {\n  for (const s of names) {\n    s.get();\n  }\n}\n\
         function b(s: Store) {\n  {\n    const s = make();\n    s.get();\n  }\n}\n\
         function once(s: Store) {\n  s.get();\n}\n",
    )
    .unwrap();
    let methods: Vec<(u64, &str)> = calls(&ts)
        .into_iter()
        .filter(|(_, name, _)| name.contains('.'))
        .map(|(line, name, _)| (line, name))
        .collect();
    assert_eq!(methods, [(13, "Store.get")], "{:?}", calls(&ts));
}

#[test]
fn a_language_not_read_inside_files_gives_nothing() {
    assert_eq!(parse("python", "x.py", "import os\n"), None);
    assert!(default_effects("python").is_empty());
    for language in SUPPORTED {
        assert!(!default_effects(language).is_empty(), "{language}");
        assert!(
            default_effects(language)
                .iter()
                .all(|(_, kind)| KINDS.contains(kind)),
            "{language}"
        );
    }
}
