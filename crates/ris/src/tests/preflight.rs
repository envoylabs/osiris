use super::*;

fn checks(toml: &str) -> ChecksConfig {
    ChecksConfig::parse(toml).unwrap()
}

fn runner(write: &[&str], network: &[&str]) -> RunnerConfig {
    RunnerConfig {
        write: write.iter().map(|s| (*s).to_owned()).collect(),
        network: network.iter().map(|s| (*s).to_owned()).collect(),
        ..RunnerConfig::default()
    }
}

/// A directory holding an executable named `name`, as a PATH.
fn path_with(name: &str) -> (PathBuf, String) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ris-preflight-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), "").unwrap();
    let p = dir.display().to_string();
    (dir, p)
}

#[test]
fn a_cargo_check_needs_its_caches_and_the_registry() {
    let c = checks("[checks.test]\ncommand = \"cargo test --workspace\"\n");
    let (_d, path) = path_with("cargo");
    let missing = check(&c, &runner(&[], &[]), Some(&path));
    assert_eq!(
        missing,
        vec![
            Missing::Write {
                tool: "cargo".into(),
                path: "~/.cargo/registry".into()
            },
            Missing::Write {
                tool: "cargo".into(),
                path: "~/.cargo/git".into()
            },
            Missing::Network {
                tool: "cargo".into(),
                host: "index.crates.io".into()
            },
            Missing::Network {
                tool: "cargo".into(),
                host: "static.crates.io".into()
            },
        ]
    );
    let text = render(&missing);
    assert!(
        text.contains("write = [\"~/.cargo/registry\", \"~/.cargo/git\"]"),
        "{text}"
    );
    assert!(
        text.contains("network = [\"index.crates.io\", \"static.crates.io\"]"),
        "{text}"
    );
    // A directory above the caches covers both; a wildcard, both hosts.
    assert!(check(&c, &runner(&["~/.cargo"], &["*.crates.io"]), Some(&path)).is_empty());
    // A bare domain does not cover its subdomains.
    assert_eq!(
        check(&c, &runner(&["~/.cargo/**"], &["crates.io"]), Some(&path)).len(),
        2
    );
}

#[test]
fn a_program_not_on_path_is_named() {
    let c = checks("[checks.lint]\ncommand = \"RUST_LOG=off nosuchtool --check\"\n");
    let (_d, path) = path_with("cargo");
    assert_eq!(
        check(&c, &runner(&[], &[]), Some(&path)),
        vec![Missing::Program {
            check: "lint".into(),
            program: "nosuchtool".into()
        }]
    );
}

#[test]
fn every_command_in_a_check_is_read() {
    assert_eq!(
        programs_in("cd web && npm ci | tee log; /usr/bin/env true"),
        ["cd", "npm", "tee", "env"]
    );
    let c = checks("[checks.web]\ncommand = \"cd web && npm test\"\n");
    // `cd` is a builtin: `npm` is the program looked for.
    let (_d, path) = path_with("npm");
    let missing = check(&c, &runner(&["~/.npm"], &[]), Some(&path));
    assert_eq!(
        missing,
        vec![Missing::Network {
            tool: "npm".into(),
            host: "registry.npmjs.org".into()
        }]
    );
}

#[test]
fn a_check_with_no_known_tool_needs_nothing_but_its_program() {
    let c = checks("[checks.ok]\ncommand = \"make check\"\n");
    let (_d, path) = path_with("make");
    assert!(check(&c, &runner(&[], &[]), Some(&path)).is_empty());
    // Nothing on PATH: `make` is named.
    assert_eq!(check(&c, &runner(&[], &[]), Some("")).len(), 1);
}

#[test]
fn covers_reads_home_and_absolute_paths_alike() {
    let home = Path::new("/h");
    assert!(covers("~/.cargo", "~/.cargo/registry", Some(home)));
    assert!(covers("/h/.cargo", "~/.cargo/registry", Some(home)));
    assert!(covers("~", "~/.npm", Some(home)));
    assert!(!covers("~/.cargo/registry", "~/.cargo/git", Some(home)));
    assert!(!covers("~/.car", "~/.cargo/git", Some(home)));
}
