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
            (8, "save", None),
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
            (6, "save", None),
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
            (7, "Save", None),
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
