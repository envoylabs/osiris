//! The committed schema against what `ris` really prints: every
//! command is run with `--json` in a scratch project, and every
//! envelope is validated against `docs/json.schema.json` with its
//! objects closed, so a value of the wrong shape or a key no type
//! declares fails. The run then checks that it left no command of the
//! schema out, and that each error kind a run can reach was seen.
//!
//! A fake agent program on PATH stands in for the agent tool.

mod common;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// Waits until told to stop, as an agent at work does.
const AGENT: &str = "#!/bin/sh\nprintf 'hello from %s\\n' \"$RIS_AGENT\"\n\
    while [ ! -f \"$RIS_ROOT/.osiris/stop-$RIS_AGENT\" ]; do sleep 0.05; done\nexit 0\n";

struct Run {
    base: PathBuf,
    repo: PathBuf,
    /// The commands whose successful output was validated.
    commands: RefCell<BTreeSet<String>>,
    /// The error kinds whose envelope was validated.
    errors: RefCell<BTreeSet<String>>,
}

impl Run {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("ris-schema-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(base.join("bin")).unwrap();
        let fake = base.join("bin/fake-agent");
        fs::write(&fake, AGENT).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            base,
            repo,
            commands: RefCell::default(),
            errors: RefCell::default(),
        }
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(["-c", "user.name=ris", "-c", "user.email=ris@localhost"])
            .args(args)
            .current_dir(&self.repo)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn command(&self, dir: &Path, agent: Option<&str>) -> Command {
        let path = format!(
            "{}:{}",
            self.base.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut c = Command::new(env!("CARGO_BIN_EXE_ris"));
        c.current_dir(dir)
            .envs(common::GIT_IDENTITY)
            .env_remove("RIS_ROOT")
            .env_remove("RIS_AGENT")
            .env("NO_COLOR", "1")
            .env("PATH", path);
        if let Some(agent) = agent {
            c.env("RIS_AGENT", agent);
        }
        c
    }

    /// Run `ris --json <args>` in `dir` and validate everything it
    /// printed; the last envelope.
    fn run_in(&self, dir: &Path, agent: Option<&str>, args: &[&str]) -> (Output, Value) {
        let out = self
            .command(dir, agent)
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout.clone()).unwrap();
        common::validate_output(args, &stdout);
        let last: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| {
            serde_json::from_str(stdout.lines().last().unwrap_or("null")).unwrap()
        });
        (out, last)
    }

    /// A command that succeeds, as `agent` or the person: its data.
    fn ok(&self, agent: Option<&str>, args: &[&str]) -> Value {
        let (out, v) = self.run_in(&self.repo, Some(agent.unwrap_or("person")), args);
        assert!(
            out.status.success() && v["ok"] == true,
            "ris {}: {v:#} {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        self.commands
            .borrow_mut()
            .insert(common::command_of(args).expect("a command of the schema"));
        v["data"].clone()
    }

    /// A command that fails with an error of `kind`: its error.
    fn fails(&self, dir: &Path, agent: Option<&str>, args: &[&str], kind: &str) -> Value {
        let (out, v) = self.run_in(dir, agent, args);
        assert!(!out.status.success(), "ris {}: {v:#}", args.join(" "));
        assert_eq!(v["error"]["kind"], kind, "ris {}: {v:#}", args.join(" "));
        self.errors.borrow_mut().insert(kind.to_owned());
        v["error"].clone()
    }

    fn config(&self) -> PathBuf {
        self.repo.join(".osiris/config.toml")
    }
}

#[test]
fn every_command_s_real_output_fits_the_committed_schema() {
    let r = Run::new();
    let heron = Some("heron");
    let wren = Some("wren");
    let person = Some("person");
    r.git(&["init", "-q", "-b", "main"]);
    fs::write(r.repo.join("README"), "base\n").unwrap();
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "base"]);

    // Before there is a project.
    r.fails(&r.repo, person, &["ready"], "not_a_project");
    r.ok(None, &["schema"]);
    r.ok(None, &["rules"]);
    r.ok(None, &["init", "--local"]);
    r.fails(&r.repo, person, &["init", "--local"], "already_a_project");
    r.fails(&r.repo, None, &["ready"], "no_actor");

    fs::write(
        r.repo.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n[checks.bad]\ncommand = \"false\"\n",
    )
    .unwrap();
    fs::write(
        r.config(),
        "denylist = [\"example client\"]\ndocs_item = false\nphase_branches = false\n\
         [runner]\nprogram = \"fake-agent\"\npreflight = false\n\
         [agents.heron]\nrole = \"lead\"\n[agents.wren]\nrole = \"auditor\"\n",
    )
    .unwrap();
    r.ok(None, &["seat", "add", "rook", "--role", "builder"]);

    // A phase, its items and what is said about them.
    r.ok(None, &["phase", "open", "fit the schema"]);
    r.ok(None, &["report"]);
    r.ok(None, &["item", "add", "the work", "--check", "ok"]);
    r.ok(None, &["item", "add", "to be replaced", "--check", "ok"]);
    r.ok(None, &["item", "add", "the replacement", "--check", "ok"]);
    r.ok(None, &["item", "add", "cannot pass", "--check", "bad"]);
    r.ok(
        None,
        &["item", "add", "waits", "--check", "ok", "--depends", "I-1"],
    );
    r.ok(None, &["item", "supersede", "I-2", "--by", "I-3"]);
    r.fails(
        &r.repo,
        person,
        &["item", "add", "no such check", "--check", "nope"],
        "config",
    );
    r.ok(None, &["ready"]);
    r.ok(heron, &["claim", "I-1"]);
    r.ok(heron, &["release", "I-1"]);
    let brief = r.ok(heron, &["claim", "I-1"]);
    assert_eq!(brief["item"], "I-1", "{brief}");
    r.fails(&r.repo, heron, &["claim", "I-99"], "refused");
    r.fails(&r.repo, wren, &["claim", "I-3"], "role_forbids");
    r.fails(
        &r.repo,
        heron,
        &["send", "nobody", "--kind", "question", "who?"],
        "unknown_agent",
    );
    r.fails(&r.repo, person, &["show", "banana"], "other");

    // Decisions: one taken as it is raised, one for the person with
    // its case, which sets an item aside until it is taken.
    r.ok(
        heron,
        &[
            "decision",
            "raise",
            "--about",
            "I-1",
            "--question",
            "Which shape?",
            "--option",
            "a",
            "--option",
            "b",
            "--rationale",
            "cheap to change",
        ],
    );
    r.ok(
        heron,
        &[
            "decision",
            "raise",
            "--substantive",
            "--about",
            "I-1",
            "--question",
            "Which engine?",
            "--option",
            "one",
            "--option",
            "two",
            "--recommend",
            "1",
        ],
    );
    r.ok(
        heron,
        &[
            "decision",
            "explain",
            "D-2",
            "--summary",
            "Where it lives",
            "--pro",
            "0: small",
            "--con",
            "0: slow",
            "--pro",
            "1: fast",
            "--why",
            "Speed matters here",
        ],
    );
    r.ok(None, &["eli5", "D-2"]);
    r.ok(None, &["eli5", "D-1"]);
    let queue = r.ok(None, &["decide"]);
    assert_eq!(queue["queue"][0]["decision"]["id"], "D-2", "{queue}");
    let aside = r.ok(None, &["show", "I-1"]);
    assert_eq!(aside["blocked_on"][0]["kind"], "decision", "{aside}");
    let taken = r.ok(
        None,
        &[
            "decide",
            "D-2",
            "--other",
            "a third",
            "--rationale",
            "neither",
        ],
    );
    assert_eq!(taken["unblocked"], serde_json::json!(["I-1"]), "{taken}");
    r.ok(heron, &["claim", "I-1"]);

    // Person actions: one that closes itself by its check, one done by
    // hand that frees the item waiting on it.
    r.ok(
        None,
        &[
            "action",
            "raise",
            "look at it",
            "--done-when",
            "looked",
            "--informs",
            "I-1",
        ],
    );
    r.ok(
        None,
        &[
            "action",
            "raise",
            "grant access",
            "--done-when",
            "granted",
            "--blocks",
            "I-3",
        ],
    );
    r.ok(None, &["inbox"]);
    r.ok(None, &["show", "A-2"]);
    r.ok(None, &["action", "check", "A-1", "--command", "true"]);
    let done = r.ok(None, &["action", "done", "A-2", "--note", "granted"]);
    assert_eq!(done["ready"], serde_json::json!(["I-3"]), "{done}");
    r.ok(None, &["show", "A-1"]);
    r.ok(None, &["show", "A-2"]);

    // Messages and inboxes.
    r.ok(
        heron,
        &[
            "send", "wren", "--kind", "question", "--about", "I-1", "settled?",
        ],
    );
    r.ok(heron, &["inbox"]);
    r.ok(wren, &["inbox"]);
    r.ok(None, &["inbox"]);
    r.ok(None, &["show", "M-1"]);

    // Checks, done, merge: one commit in the main checkout.
    r.ok(None, &["check", "ok"]);
    r.fails(&r.repo, person, &["check", "bad"], "check_failed");
    r.ok(heron, &["claim", "I-4"]);
    r.fails(&r.repo, heron, &["done", "I-4"], "checks_failed");
    r.ok(heron, &["release", "I-4"]);
    // A merge runs every declared check, so the failing one goes.
    fs::write(
        r.repo.join(".osiris/checks.toml"),
        "[checks.ok]\ncommand = \"true\"\n",
    )
    .unwrap();
    fs::create_dir_all(r.repo.join("src")).unwrap();
    fs::write(
        r.repo.join("src/lib.rs"),
        "use std::fs;\n\npub fn read() -> bool {\n    fs::metadata(\"x\").is_ok()\n}\n",
    )
    .unwrap();
    r.git(&["add", "-A"]);
    r.git(&[
        "commit",
        "-q",
        "-m",
        "feat: the work",
        "-m",
        "Item: I-1\nAgent: heron",
    ]);
    let done = r.ok(heron, &["done", "I-1"]);
    let evidence = done["evidence"][0].as_str().expect("evidence").to_owned();
    r.ok(heron, &["merge", "I-1"]);
    r.ok(None, &["show", &evidence]);
    r.ok(None, &["show", "I-1"]);
    r.ok(None, &["show", "D-1"]);
    r.ok(None, &["show", "D-2"]);
    r.ok(None, &["show", "P-1"]);

    // Review.
    let queue = r.ok(wren, &["audit"]);
    assert_eq!(queue["queue"][0]["id"], "I-1", "{queue}");
    r.ok(wren, &["audit", "I-1"]);
    // A risk judgement is of a file the phase changed. With phase
    // branches off the work is on the main line itself, which the map
    // compares with nothing, so the change judged here is made on a
    // branch beside it and dropped again.
    r.git(&["switch", "-q", "-c", "side"]);
    fs::write(
        r.repo.join("src/lib.rs"),
        "pub fn read() -> bool {\n    false\n}\n",
    )
    .unwrap();
    r.git(&["commit", "-q", "-a", "-m", "wip"]);
    r.fails(
        &r.repo,
        heron,
        &[
            "audit",
            "--file",
            "src/lib.rs",
            "--risk",
            "high",
            "--why",
            "mine",
        ],
        "role_forbids",
    );
    let judged = r.ok(
        wren,
        &[
            "audit",
            "--file",
            "src/lib.rs",
            "--risk",
            "high",
            "--why",
            "drops the read",
        ],
    );
    assert_eq!(judged["level"], "high", "{judged}");
    let map = r.ok(None, &["map"]);
    assert_eq!(map["nodes"][0]["risk"]["judged"]["seat"], "wren", "{map}");
    r.git(&["switch", "-q", "main"]);
    r.git(&["branch", "-q", "-D", "side"]);
    r.ok(None, &["summary"]);
    r.ok(None, &["summary", "--all"]);
    r.ok(heron, &["summary"]);
    r.ok(None, &["report"]);
    r.ok(None, &["status"]);
    r.ok(None, &["verify"]);
    let map = r.ok(None, &["map"]);
    assert_eq!(map["nodes"][0]["path"], "src/lib.rs", "{map}");
    r.ok(None, &["scan", "src"]);
    // The canary is assembled at runtime: the exit scan has no
    // exceptions, so the literal is never in the tree.
    fs::write(
        r.repo.join("notes.md"),
        format!("see {}\n", ["/Us", "ers/someone/x"].concat()),
    )
    .unwrap();
    r.fails(&r.repo, person, &["scan", "notes.md"], "leak");
    fs::remove_file(r.repo.join("notes.md")).unwrap();

    // Import, dry and for real.
    fs::write(
        r.repo.join(".osiris/backlog.toml"),
        "[[item]]\ngoal = \"Imported work\"\nchecks = [\"ok\"]\n\n\
         [[decision]]\nquestion = \"Which store?\"\noptions = [\"file\", \"table\"]\n\
         answer = 0\nrationale = \"Settled before\"\n\n\
         [[decision]]\nquestion = \"Which runner?\"\noptions = [\"one\", \"two\"]\n\
         recommend = 0\nrationale = \"Open\"\n",
    )
    .unwrap();
    r.ok(None, &["import", ".osiris/backlog.toml", "--dry-run"]);
    r.ok(None, &["import", ".osiris/backlog.toml"]);

    // The person's ris, installed from the last merge. The table is
    // there for this one command: with it, every command of the
    // person's first catches up with the last merge.
    let plain = fs::read_to_string(r.config()).unwrap();
    fs::write(
        r.config(),
        format!(
            "{plain}\n[install]\nbuild = \"mkdir -p out && git rev-parse HEAD > out/ris\"\n\
             artifact = \"out/ris\"\nto = {:?}\ndir = {:?}\n",
            r.base.join("installed/ris").to_string_lossy(),
            r.base.join("build").to_string_lossy(),
        ),
    )
    .unwrap();
    r.ok(None, &["install"]);
    fs::write(r.config(), plain).unwrap();

    // Seats: a worktree, a run in the foreground, the harness up, a
    // stop recorded by hand, pauses and resumes, the logs.
    r.ok(None, &["worktree", "rook"]);
    r.ok(heron, &["seat", "fault", "cannot reach the registry"]);
    let stop = |seat: &str| fs::write(r.repo.join(format!(".osiris/stop-{seat}")), "").unwrap();
    stop("rook");
    let stopped = r.ok(None, &["seat", "start", "rook", "--quiet"]);
    assert_eq!(stopped["status"], 0, "{stopped}");
    fs::remove_file(r.repo.join(".osiris/stop-rook")).unwrap();
    r.ok(None, &["logs", "rook"]);
    r.ok(None, &["up"]);
    r.ok(None, &["status"]);
    r.ok(None, &["seat", "stopped", "heron"]);
    r.ok(None, &["logs"]);
    r.ok(None, &["seat", "pause", "rook"]);
    r.ok(None, &["seat", "resume", "--all"]);
    r.ok(None, &["down"]);
    for seat in ["heron", "rook", "wren"] {
        stop(seat);
    }

    // Close, then what reads the closed phase and the log.
    r.ok(None, &["phase", "close"]);
    r.ok(None, &["report"]);
    r.ok(None, &["export", r.base.join("export").to_str().unwrap()]);
    r.ok(None, &["log"]);
    r.ok(None, &["watch", "--from", "1", "--once"]);

    // A next phase, and an item from the backlog brought into it.
    r.ok(None, &["phase", "open", "the next"]);
    let ready = r.ok(None, &["ready"]);
    let backlog = ready["ready"][0]["id"].as_str().expect("a backlog item");
    r.ok(None, &["item", "bring-in", backlog]);

    // Nothing was left out: every command the schema has data for was
    // run and validated here.
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/json.schema.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let all: BTreeSet<String> = schema["$defs"]
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|k| k.strip_prefix("data."))
        .map(|k| k.to_owned())
        .collect();
    let seen: BTreeSet<String> = r
        .commands
        .borrow()
        .iter()
        .map(|c| c.replace(' ', "_"))
        .collect();
    assert_eq!(
        all.difference(&seen).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "commands of the schema this run never validated"
    );
    // Every error kind a run can bring about. `contended`, `store` and
    // `io` need a fault in the machine and are not provoked here.
    let kinds: BTreeSet<String> = schema["$defs"]["ErrorKind"]["enum"]
        .as_array()
        .expect("the error kinds")
        .iter()
        .map(|k| k.as_str().unwrap().to_owned())
        .filter(|k| !["contended", "store", "io"].contains(&k.as_str()))
        .collect();
    assert_eq!(*r.errors.borrow(), kinds);
}

/// The validation itself is not vacuous: a value of the wrong type, a
/// key no type declares and a missing key are each refused.
#[test]
fn a_document_that_does_not_fit_is_refused() {
    let fits = serde_json::json!({
        "format": 1, "ok": true,
        "data": { "len": 3, "head": "0".repeat(64), "missing_text": [] }
    });
    common::validate(&["verify"], &fits);
    let refused = |envelope: Value| {
        std::panic::catch_unwind(|| common::validate(&["verify"], &envelope)).is_err()
    };
    let with = |change: &dyn Fn(&mut Value)| {
        let mut v = fits.clone();
        change(&mut v);
        v
    };
    assert!(
        refused(with(&|v| v["data"]["len"] = "three".into())),
        "a wrong type"
    );
    assert!(
        refused(with(&|v| v["data"]["head"] = "abc".into())),
        "a hash that is not one"
    );
    assert!(
        refused(with(&|v| v["data"]["extra"] = 1.into())),
        "an undeclared key"
    );
    assert!(
        refused(with(&|v| {
            v["data"].as_object_mut().unwrap().remove("head");
        })),
        "a missing key"
    );
    assert!(refused(with(&|v| v["format"] = 2.into())), "another format");
    assert!(
        refused(with(&|v| v["note"] = "x".into())),
        "a key on the envelope"
    );
}
