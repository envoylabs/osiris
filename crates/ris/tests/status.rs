//! `ris status`: each seat's worktree, branch, claim, divergence from the
//! phase branch and uncommitted changes, from git and the log.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}", args.join(" "));
}

fn ris(dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ris"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("RIS_AGENT")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    if let Some(a) = agent {
        cmd.env("RIS_AGENT", a);
    }
    cmd.output().unwrap()
}

fn ok(dir: &Path, agent: Option<&str>, args: &[&str]) -> String {
    let out = ris(dir, agent, args);
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn status_shows_where_each_seat_is() {
    let base = std::env::temp_dir().join(format!("ris-status-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main = base.join("proj");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    fs::write(main.join("README"), "base\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    ok(&main, None, &["init", "--local"]);
    let config = main.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]"),
    )
    .unwrap();
    ok(&main, None, &["seat", "add", "rook", "--role", "builder"]);
    ok(&main, None, &["seat", "add", "kite", "--role", "builder"]);
    ok(&main, None, &["phase", "open", "status"]);
    ok(&main, None, &["item", "add", "a task"]);
    ok(&main, None, &["worktree", "rook"]);
    let rook = base.join("proj-rook");
    ok(&rook, Some("rook"), &["claim", "I-1"]);
    fs::write(rook.join("a.txt"), "a\n").unwrap();
    git(&rook, &["add", "-A"]);
    git(&rook, &["commit", "-q", "-m", "wip"]);
    fs::write(rook.join("b.txt"), "b\n").unwrap();

    let out = ok(&main, None, &["--json", "status"]);
    assert!(
        !out.contains(base.to_str().unwrap()),
        "no absolute path: {out}"
    );
    let v = serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"].clone();
    let seat = |name: &str| {
        v["seats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["seat"] == name)
            .unwrap()
            .clone()
    };
    let r = seat("rook");
    assert_eq!(r["worktree"], "../proj-rook", "{r}");
    assert_eq!(r["branch"], "rook");
    assert_eq!(r["claim"], "I-1");
    assert_eq!(
        (r["ahead"].clone(), r["behind"].clone()),
        (1.into(), 0.into())
    );
    assert_eq!(r["uncommitted"], 1);

    let k = seat("kite");
    assert!(k["worktree"].is_null() && k["claim"].is_null(), "{k}");
    for name in ["heron", "wren"] {
        let s = seat(name);
        assert_eq!(s["worktree"], "main checkout", "{s}");
        assert_eq!(s["branch"], "phase/P-1", "{s}");
    }

    let text = ok(&main, None, &["status"]);
    assert!(text.contains("no worktree (ris worktree kite)"), "{text}");
    assert!(
        text.contains("+1 -0") && text.contains("holds I-1"),
        "{text}"
    );
}
