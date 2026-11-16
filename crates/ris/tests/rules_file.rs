//! A rules file is rendered, never written by hand: `ris rules
//! --render` gives it from the rules template and the project's own
//! part, and only the person runs it. Nothing here reads this
//! repository's own rules file, so a seat that changes the rules or the
//! project's part passes its checks with the file not yet rendered:
//! the closing merge of the phase renders it, and `ris rules --render
//! ... --check`, which CI runs on the main line, says whether a file is
//! current.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The project's own part and the rules file, from the repository root.
const SOURCE: &str = "docs/project-rules.md";
const RULES_FILE: &str = "AGENTS.md";

fn ris(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ris"))
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .env("RIS_AGENT", agent)
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn ok(dir: &Path, agent: &str, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(common::GIT_IDENTITY)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A directory of the test's own, named in its canonical form.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ris-rules-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("docs")).unwrap();
    dir
}

/// `--check` writes nothing and says whether the file is current, for
/// a seat as for the person: exit 3 and the first differing line when
/// it is not.
#[test]
fn the_check_says_whether_a_rules_file_is_current_and_writes_nothing() {
    let dir = scratch("check");
    ok(&dir, "person", &["init"]);
    fs::write(dir.join(SOURCE), "# Ours\n\nOne rule of our own.\n").unwrap();
    let check = ["rules", "--render", SOURCE, "--to", RULES_FILE, "--check"];

    // No rules file yet: an error, and the check does not make one.
    assert!(!ris(&dir, "heron", &check).status.success());
    assert!(!dir.join(RULES_FILE).exists());

    ok(
        &dir,
        "person",
        &["rules", "--render", SOURCE, "--to", RULES_FILE],
    );
    for who in ["person", "heron", "wren"] {
        let text = ok(&dir, who, &check);
        assert!(
            text.contains("AGENTS.md is what the rules and docs/project-rules.md render"),
            "{text}"
        );
    }
    let out = ris(
        &dir,
        "heron",
        &[
            "--json", "rules", "--render", SOURCE, "--to", RULES_FILE, "--check",
        ],
    );
    common::validate_output(&check, &String::from_utf8_lossy(&out.stdout));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["current"], true, "{v}");
    assert_eq!(v["data"]["checked"], RULES_FILE);

    // The project's part changes and the file is not rendered again.
    fs::write(dir.join(SOURCE), "# Ours\n\nAnother rule.\n").unwrap();
    let before = fs::read_to_string(dir.join(RULES_FILE)).unwrap();
    let out = ris(&dir, "heron", &check);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("AGENTS.md is not what the rules and docs/project-rules.md render"),
        "{err}"
    );
    assert!(
        err.contains("in the file: \"One rule of our own.\""),
        "{err}"
    );
    assert!(err.contains("rendered:    \"Another rule.\""), "{err}");
    assert!(
        err.contains("the person runs `ris rules --render docs/project-rules.md --to AGENTS.md`"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(dir.join(RULES_FILE)).unwrap(), before);
    let out = ris(
        &dir,
        "heron",
        &[
            "--json", "rules", "--render", SOURCE, "--to", RULES_FILE, "--check",
        ],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "check_failed", "{v}");
    let _ = fs::remove_dir_all(&dir);
}

/// A phase that changes the project's part closes with the rules file
/// rendered into the closing merge, with no step of the person's; one
/// that changes nothing renders nothing; and a close whose checks fail
/// leaves the main line, and its rules file, where they were.
#[test]
fn closing_a_phase_renders_the_rules_file_into_the_closing_merge() {
    let dir = scratch("close");
    git(&dir, &["init", "-q", "-b", "main"]);
    ok(&dir, "person", &["init", "--local"]);
    // The checks pass until a file named `stop` is in the tree.
    fs::write(
        dir.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"test ! -e stop\"\n",
    )
    .unwrap();
    fs::write(
        dir.join(".osiris/config.toml"),
        "denylist = [\"example client\"]\ndocs_item = false\n[agents.heron]\nrole = \"lead\"\n",
    )
    .unwrap();
    fs::write(dir.join(SOURCE), "# Ours\n\nThe first rule.\n").unwrap();
    ok(
        &dir,
        "person",
        &["rules", "--render", SOURCE, "--to", RULES_FILE],
    );
    // A rules file written by hand, which no close may touch.
    fs::write(dir.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    let rendered =
        |part: &str| ris::rules_file::render(SOURCE, RULES_FILE, &format!("# Ours\n\n{part}\n"));
    let on_main = |file: &str| git(&dir, &["show", &format!("main:{file}")]) + "\n";
    // What a seat's merge leaves on the phase branch: `files` changed,
    // and the rules file as it was.
    let phase_commit = |branch: &str, files: &[(&str, &str)]| {
        git(&dir, &["switch", "-q", branch]);
        for (file, text) in files {
            fs::write(dir.join(file), text).unwrap();
        }
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "feat: change"]);
        git(&dir, &["rev-parse", "HEAD"])
    };

    // The phase changes the project's part and never renders.
    ok(&dir, "person", &["phase", "open", "one"]);
    let tip = phase_commit("phase/P-1", &[(SOURCE, "# Ours\n\nThe second rule.\n")]);
    assert_eq!(
        fs::read_to_string(dir.join(RULES_FILE)).unwrap(),
        rendered("The first rule.")
    );
    let out = ok(&dir, "person", &["phase", "close"]);
    assert!(
        out.contains(
            "closing merge: rendered AGENTS.md from the rules and docs/project-rules.md, which the phase changed; the merge carries it"
        ),
        "{out}"
    );
    // The main line holds the file as rendered, in the merge commit
    // itself: one commit past the phase's, differing from it in the
    // rules file alone, with nothing left uncommitted.
    assert_eq!(on_main(RULES_FILE), rendered("The second rule."));
    assert_eq!(git(&dir, &["rev-parse", "main^2"]), tip);
    assert_eq!(
        git(&dir, &["diff", "--name-only", "main^2", "main"]),
        RULES_FILE
    );
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
    assert_eq!(on_main("CLAUDE.md"), "@AGENTS.md\n");
    ok(
        &dir,
        "heron",
        &["rules", "--render", SOURCE, "--to", RULES_FILE, "--check"],
    );

    // A phase that leaves the rules alone renders nothing.
    ok(&dir, "person", &["phase", "open", "two"]);
    phase_commit("phase/P-2", &[("a.txt", "a\n")]);
    let out = ok(&dir, "person", &["phase", "close"]);
    assert!(out.contains("closing merge: phase/P-2 into main"), "{out}");
    assert!(!out.contains("rendered"), "{out}");
    assert_eq!(git(&dir, &["diff", "--name-only", "main^2", "main"]), "");

    // A close refused by a failing check undoes the merge and the
    // rendering with it.
    ok(&dir, "person", &["phase", "open", "three"]);
    phase_commit(
        "phase/P-3",
        &[(SOURCE, "# Ours\n\nThe third rule.\n"), ("stop", "")],
    );
    let before = git(&dir, &["rev-parse", "main"]);
    let out = ris(&dir, "person", &["phase", "close"]);
    assert!(!out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("the merge was undone"), "{err}");
    assert_eq!(git(&dir, &["rev-parse", "main"]), before);
    assert_eq!(on_main(RULES_FILE), rendered("The second rule."));
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
    let _ = fs::remove_dir_all(&dir);
}

/// The command writes that same file, for the person and nobody else.
#[test]
fn the_person_renders_a_rules_file_and_a_seat_is_refused() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ris-rules-render-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("docs")).unwrap();
    assert!(ris(&dir, "person", &["init"]).status.success());
    fs::write(dir.join(SOURCE), "# Ours\n\nOne rule of our own.\n").unwrap();
    let render = ["rules", "--render", SOURCE, "--to", RULES_FILE];

    // A seat never writes its own rules, whatever its role.
    for seat in ["heron", "wren"] {
        let out = ris(&dir, seat, &render);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{seat} rendered the rules");
        assert!(err.contains("only the person renders"), "{err}");
        assert!(!dir.join(RULES_FILE).exists());
    }

    let out = ris(&dir, "person", &render);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("rendered AGENTS.md from the rules and docs/project-rules.md"),
        "{text}"
    );
    let written = fs::read_to_string(dir.join(RULES_FILE)).unwrap();
    assert_eq!(
        written,
        ris::rules_file::render(SOURCE, RULES_FILE, "# Ours\n\nOne rule of our own.\n")
    );
    // The rules ris prints are in it whole, and the project's part.
    let rules = String::from_utf8(ris(&dir, "person", &["rules"]).stdout).unwrap();
    assert!(written.contains(rules.trim_end()), "the template, whole");
    assert!(written.ends_with("One rule of our own.\n"), "{written}");

    // Again changes nothing, and says so.
    let out = ris(
        &dir,
        "person",
        &["--json", "rules", "--render", SOURCE, "--to", RULES_FILE],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["changed"], false, "{v}");
    assert_eq!(v["data"]["rendered"], RULES_FILE);

    // A path outside the directory is refused, and nothing is written
    // there. Built from pieces: no rooted path stands in the tree.
    let outside = format!("..{}escaped.md", '/');
    let out = ris(
        &dir,
        "person",
        &["rules", "--render", SOURCE, "--to", &outside],
    );
    assert!(!out.status.success());
    assert!(!dir.with_file_name("escaped.md").exists());
    let _ = fs::remove_dir_all(&dir);
}
