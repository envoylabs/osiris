//! `ris watch` follows the log: a second process appending events is
//! seen by a running watcher, each event verified and printed as one
//! JSON line.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn cmd(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ris"));
    c.current_dir(dir)
        .env("RIS_AGENT", "person")
        .env_remove("RIS_ROOT")
        .env("NO_COLOR", "1");
    c
}

fn ok(dir: &Path, args: &[&str]) -> Output {
    let out = cmd(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "ris {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn a_watcher_sees_what_another_process_appends() {
    let dir = std::env::temp_dir().join(format!("ris-watch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    ok(&dir, &["phase", "open", "watching"]);

    // Replaying from the start returns what is there and exits.
    let out = ok(&dir, &["watch", "--from", "1", "--once"]);
    let first: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().next().unwrap()).unwrap();
    assert_eq!(first["seq"], 1);
    assert_eq!(first["structure"]["event"], "phase_opened", "{first}");

    // A running watcher sees only what comes after it started.
    let mut child = cmd(&dir)
        .args(["watch", "--interval-ms", "20"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    std::thread::sleep(Duration::from_millis(200));
    ok(&dir, &["item", "add", "first"]);
    ok(&dir, &["item", "add", "second"]);
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let line = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the watcher printed the new events");
        let e: serde_json::Value = serde_json::from_str(&line).unwrap();
        seen.push((e["seq"].as_u64().unwrap(), e["structure"]["event"].clone()));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        seen,
        vec![(2, "item_added".into()), (3, "item_added".into())],
        "only the new events, in order"
    );

    // Asking to start beyond the log is an error, not a silent wait.
    let r = cmd(&dir)
        .args(["watch", "--from", "99", "--once"])
        .output()
        .unwrap();
    assert!(!r.status.success());
}
