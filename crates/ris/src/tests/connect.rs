use super::*;

fn tree(files: &[(&str, &str)]) -> Tree {
    files
        .iter()
        .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
        .collect()
}

fn edges(scan: &BTreeMap<String, Scan>, file: &str) -> Vec<(String, &'static str)> {
    scan[file].edges.iter().cloned().collect()
}

fn to(path: &str, kind: &'static str) -> (String, &'static str) {
    (path.to_owned(), kind)
}

#[test]
fn typescript_resolves_relative_imports_as_tsc_does() {
    let t = tree(&[
        (
            "src/app.ts",
            "import { save } from \"./store\";\n\
             import * as wire from \"./wire.js\";\n\
             import { ui } from \"./ui\";\n\
             import express from \"express\";\n\
             export { log } from \"../lib/log\";\n\
             const cfg = require(\"./config\");\n\
             import(\"./lazy\").then(() => {});\n\
             import missing from \"./nowhere\";\n",
        ),
        (
            "src/store.ts",
            "import fs from \"node:fs\";\nfs.readFileSync(\"x\");\n",
        ),
        ("src/wire.ts", "export const w = 1;\n"),
        ("src/ui/index.tsx", "export const ui = 1;\n"),
        ("src/config.js", "module.exports = {};\n"),
        ("src/lazy.ts", "export {};\n"),
        ("lib/log.ts", "export const log = 1;\n"),
    ]);
    let scan = TypeScript.scan(&t);
    assert_eq!(
        edges(&scan, "src/app.ts"),
        vec![
            to("lib/log.ts", "import"),
            to("src/config.js", "import"),
            to("src/lazy.ts", "import"),
            to("src/store.ts", "import"),
            to("src/ui/index.tsx", "import"),
            to("src/wire.ts", "import"),
        ]
    );
    // A package and a missing file are no edge; a module the patterns
    // name is how the file touches the outside.
    assert!(scan["src/app.ts"].external.is_empty());
    assert_eq!(
        scan["src/store.ts"]
            .external
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec!["filesystem"]
    );
    // A JavaScript file is a target, not a scanned file.
    assert!(!scan.contains_key("src/config.js"));
}

#[test]
fn typescript_never_climbs_above_the_root() {
    let t = tree(&[("a.ts", "import x from \"../b\";\n"), ("b.ts", "")]);
    assert!(TypeScript.scan(&t)["a.ts"].edges.is_empty());
}

#[test]
fn go_connects_a_file_to_every_file_of_the_package_it_imports() {
    let t = tree(&[
        ("go.mod", "module example.com/shop\n\ngo 1.22\n"),
        (
            "cmd/shop/main.go",
            "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/shop/store\"\n\tw \"example.com/shop/internal/wire\"\n)\n\nfunc main() { fmt.Println(store.Load(), w.X) }\n",
        ),
        (
            "store/store.go",
            "package store\n\nimport \"os\"\n\nfunc Load() string { b, _ := os.ReadFile(\"x\"); return string(b) }\n",
        ),
        ("store/cache.go", "package store\n"),
        ("store/store_test.go", "package store\n"),
        ("internal/wire/wire.go", "package wire\n\nconst X = 1\n"),
        // A module of its own, nested: its paths are its own.
        ("tools/go.mod", "module example.com/tools\n"),
        (
            "tools/gen.go",
            "package main\n\nimport \"example.com/shop/store\"\n\nfunc main() { store.Load() }\n",
        ),
    ]);
    let scan = Go.scan(&t);
    assert_eq!(
        edges(&scan, "cmd/shop/main.go"),
        vec![
            to("internal/wire/wire.go", "import"),
            to("store/cache.go", "import"),
            to("store/store.go", "import"),
            to("store/store_test.go", "import"),
        ]
    );
    assert_eq!(
        scan["store/store.go"]
            .external
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec!["filesystem"]
    );
    assert!(scan["cmd/shop/main.go"].external.is_empty());
    // Another module's file importing this module's path still lands.
    assert_eq!(
        edges(&scan, "tools/gen.go"),
        vec![
            to("store/cache.go", "import"),
            to("store/store.go", "import"),
            to("store/store_test.go", "import"),
        ]
    );
}

#[test]
fn clojure_connects_a_namespace_to_the_files_that_declare_what_it_requires() {
    let t = tree(&[
        (
            "src/shop/core.clj",
            "(ns shop.core\n  (:require [shop.store :as store]\n            [clojure.string :as str]\n            shop.wire)\n  (:use [shop.util]))\n\n(defn run [] (store/load))\n",
        ),
        (
            "src/shop/store.clj",
            "(ns shop.store (:require [clojure.java.io :as io]))\n(defn load [] (slurp (io/file \"x\")))\n",
        ),
        ("src/shop/wire.cljc", "(ns shop.wire)\n"),
        ("src/shop/wire.cljs", "(ns shop.wire)\n"),
        ("src/shop/util.clj", "(ns shop.util)\n"),
    ]);
    let scan = Clojure.scan(&t);
    assert_eq!(
        edges(&scan, "src/shop/core.clj"),
        vec![
            to("src/shop/store.clj", "require"),
            to("src/shop/util.clj", "require"),
            to("src/shop/wire.cljc", "require"),
            to("src/shop/wire.cljs", "require"),
        ]
    );
    assert_eq!(
        scan["src/shop/store.clj"]
            .external
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec!["filesystem"]
    );
}
