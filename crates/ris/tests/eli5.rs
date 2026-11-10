//! `ris eli5`: a decision's full case on demand, while the summary
//! screen stays terse.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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
fn a_raiser_explains_and_the_person_drills_down() {
    let dir = std::env::temp_dir().join(format!("ris-eli5-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, None, &["init"]);
    let config = dir.join(".osiris/config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("denylist = []", "denylist = [\"example client\"]")
            .replace("phase_branches = true", "phase_branches = false"),
    )
    .unwrap();
    ok(&dir, None, &["phase", "open", "eli5"]);
    let heron = Some("heron");
    let wren = Some("wren");
    ok(
        &dir,
        heron,
        &[
            "decision",
            "raise",
            "--substantive",
            "--question",
            "Which engine?",
            "--option",
            "sqlite",
            "--option",
            "rocksdb",
            "--recommend",
            "0",
        ],
    );

    // Before an explanation, eli5 says so and names who can give one.
    let bare = ok(&dir, None, &["eli5", "D-1"]);
    assert!(
        bare.contains("no explanation recorded") && bare.contains("ask heron"),
        "{bare}"
    );

    let explain = |agent: Option<&str>, extra: &[&str]| {
        let mut args = vec![
            "decision",
            "explain",
            "D-1",
            "--summary",
            "Where the log lives",
            "--why",
            "Nothing to install, and one file to back up",
        ];
        args.extend_from_slice(extra);
        ris(&dir, agent, &args)
    };
    let full = [
        "--pro",
        "0: bundled, nothing to install",
        "--pro",
        "0: one file",
        "--con",
        "0: one writer at a time",
        "--pro",
        "1: fast appends",
        "--con",
        "1: a native build",
    ];
    assert_eq!(
        explain(wren, &full).status.code(),
        Some(2),
        "not the raiser"
    );
    assert_eq!(
        explain(heron, &["--pro", "0: one file"]).status.code(),
        Some(2),
        "option 1 has nothing"
    );
    assert_eq!(
        explain(heron, &["--pro", "zero: x", "--con", "1: y"])
            .status
            .code(),
        Some(1),
        "a point must name its option by number"
    );
    let out = explain(heron, &full);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The hyphen is optional on input, for every kind of id.
    assert_eq!(
        ok(&dir, None, &["eli5", "D1"]),
        ok(&dir, None, &["eli5", "D-1"])
    );
    assert!(ok(&dir, None, &["show", "P1"]).starts_with("P-1"));
    let case = ok(&dir, None, &["eli5", "D-1"]);
    for want in [
        "Where the log lives",
        "0: sqlite  * recommended",
        "   + bundled, nothing to install",
        "   + one file",
        "   - one writer at a time",
        "1: rocksdb",
        "   + fast appends",
        "   - a native build",
        "why 0: Nothing to install, and one file to back up",
        "explained by agent heron",
        "ris decide D-1 --choose",
    ] {
        assert!(case.contains(want), "missing {want:?} in:\n{case}");
    }

    // The summary stays terse and points at the drilldown.
    let summary = ok(&dir, None, &["summary"]);
    assert!(summary.contains("the full case: ris eli5 D-1"), "{summary}");
    assert!(
        !summary.contains("fast appends"),
        "no pros and cons on the summary"
    );

    let v: serde_json::Value =
        serde_json::from_str(&ok(&dir, None, &["--json", "eli5", "D-1"])).unwrap();
    assert_eq!(
        v["explanation"]["options"][0]["pros"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(v["explanation"]["options"][1]["cons"][0], "a native build");
    assert_eq!(v["decision"]["explained"], true);

    // Taken, it can still be read, but not explained again.
    ok(
        &dir,
        None,
        &["decide", "D-1", "--choose", "0", "--rationale", "agreed"],
    );
    assert!(ok(&dir, None, &["eli5", "D-1"]).contains("taken by the person"));
    assert_eq!(explain(heron, &full).status.code(), Some(2));
}
