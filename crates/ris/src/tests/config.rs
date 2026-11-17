use super::*;

#[test]
fn the_templates_parse() {
    let c = Config::parse(Config::TEMPLATE).unwrap();
    assert_eq!(c.agents["heron"].role, Role::Lead);
    assert_eq!(c.agents["wren"].role, Role::Auditor);
    assert!(c.denylist.is_empty());
    c.check_agent_names().unwrap();
    assert_eq!(c.install, None, "installing is off until configured");
    let on = Config::parse(&Config::TEMPLATE.replace("# [install]", "[install]").replace(
            "\n# build = \"cargo build --release -p ris\"\n# artifact = \"target/release/ris\"\n# to =",
            "\nbuild = \"cargo build --release -p ris\"\nartifact = \"target/release/ris\"\nto =",
        ))
        .unwrap();
    let install = on
        .install
        .expect("the template's example, uncommented, parses");
    assert_eq!(install.artifact, "target/release/ris");
    assert_eq!(on.agents.len(), 2, "the seats below stay seats");
    let k = ChecksConfig::parse(ChecksConfig::TEMPLATE).unwrap();
    assert_eq!(k.checks["test"].command, "cargo test --workspace");
}

#[test]
fn a_denylist_filled_in_the_template_takes_effect() {
    let c =
        Config::parse(&Config::TEMPLATE.replace("denylist = []", "denylist = [\"acme\"]")).unwrap();
    assert_eq!(c.policy().denylist_len(), 1);
}

#[test]
fn a_key_in_the_wrong_table_is_an_error() {
    let misplaced = "[agents.wren]\nrole = \"auditor\"\ndenylist = [\"acme\"]\n";
    assert!(Config::parse(misplaced).is_err());
}

#[test]
fn a_seat_is_appended_with_comments_kept_and_bad_names_refused() {
    let out = Config::with_seat(Config::TEMPLATE, "rook", Role::Builder, &[]).unwrap();
    assert!(
        out.starts_with(Config::TEMPLATE.trim_end()),
        "nothing above is rewritten"
    );
    let c = Config::parse(&out).unwrap();
    assert_eq!(c.agents["rook"].role, Role::Builder);
    assert_eq!(c.agents.len(), 3);

    let refused = |name: &str, retired: &[String]| {
        Config::with_seat(Config::TEMPLATE, name, Role::Lead, retired).is_err()
    };
    assert!(refused("heron", &[]), "taken");
    assert!(refused("kite", &["kite".to_owned()]), "retired");
    for bad in ["", "Rook", "two words", "x]\ndenylist = []\n[y", "9lives"] {
        assert!(refused(bad, &[]), "{bad:?}");
    }
    let with_denylist = Config::TEMPLATE.replace("denylist = []", "denylist = [\"examplename\"]");
    assert!(
        Config::with_seat(&with_denylist, "examplename", Role::Lead, &[]).is_err(),
        "denylisted"
    );
}

#[test]
fn worktrees_default_beside_the_repository_and_can_be_placed() {
    // A root assembled from pieces: a literal absolute path in the
    // tree is refused by the exit scan.
    let root: PathBuf = [std::path::MAIN_SEPARATOR_STR, "w", "proj"]
        .iter()
        .collect();
    let base = root.parent().unwrap();
    let c = Config::parse(Config::TEMPLATE).unwrap();
    assert_eq!(c.worktrees, DEFAULT_WORKTREES);
    assert_eq!(c.worktree_path(&root, "rook"), base.join("proj-rook"));
    assert_eq!(
        Config::default().worktree_path(&root, "rook"),
        base.join("proj-rook"),
        "unset is the default"
    );

    let inside = Config::parse("worktrees = \".seats/{seat}\"\n").unwrap();
    assert_eq!(
        inside.worktree_path(&root, "kite"),
        root.join(".seats").join("kite")
    );
    let elsewhere = Config::parse("worktrees = \"../trees/{repo}/./{seat}\"\n").unwrap();
    assert_eq!(
        elsewhere.worktree_path(&root, "kite"),
        base.join("trees").join("proj").join("kite")
    );
    assert!(
        Config::parse("worktrees = \"../{repo}-shared\"\n").is_err(),
        "every seat would share one path"
    );
}

#[test]
fn a_path_is_shown_relative_without_its_prefix() {
    let root: PathBuf = [std::path::MAIN_SEPARATOR_STR, "w", "proj"]
        .iter()
        .collect();
    let base = root.parent().unwrap();
    assert_eq!(
        relative(&root, &base.join("proj-rook")),
        Path::new("../proj-rook")
    );
    assert_eq!(
        relative(&root, &root.join(".seats/kite")),
        Path::new(".seats/kite")
    );
    assert_eq!(relative(&root, &root), Path::new("."));
    assert_eq!(
        relative(&root, &base.join("a/../trees/x")),
        Path::new("../trees/x")
    );
}

#[test]
fn a_denylisted_agent_name_is_refused() {
    let c = Config::parse(
        "denylist = [\"Example Person\"]\n[agents.\"example person\"]\nrole = \"lead\"\n",
    )
    .unwrap();
    assert!(c.check_agent_names().is_err());
}

/// Each language's convention for its tests is a test file by default,
/// and the source beside it is not.
#[test]
fn the_default_test_patterns_cover_each_languages_convention() {
    let tests = default_map_tests();
    let is_test = |path: &str| tests.iter().any(|t| crate::map::glob(t, path));
    for path in [
        "tests/map.rs",
        "crates/ris/src/tests/map.rs",
        "store/store_test.go",
        "store_test.go",
        "src/app.test.ts",
        "src/ui/button.test.tsx",
        "src/app.spec.ts",
        "web/app.spec.js",
        "src/__tests__/app.ts",
        "test/shop/core_test.clj",
        "src/shop/core_test.cljs",
        "src/shop/wire_test.cljc",
    ] {
        assert!(is_test(path), "{path} is a test file");
    }
    for path in [
        "src/map.rs",
        "store/store.go",
        "store/testdata.go",
        "src/app.ts",
        "src/latest.ts",
        "src/contest.spec-helper.ts",
        "src/shop/core.clj",
        "src/shop/test_util.clj",
    ] {
        assert!(!is_test(path), "{path} is source");
    }
}
