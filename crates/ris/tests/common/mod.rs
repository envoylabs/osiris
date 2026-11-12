//! What every test that runs `ris` shares.

// Each test file uses its own part of this.
#![allow(dead_code)]

use std::sync::OnceLock;

use serde_json::{Value, json};

/// A fixed git identity for the `ris` a test runs. `ris` itself makes
/// commits (the closing merge, the rebase inside `ris merge`), and a
/// machine with no identity configured, such as a CI runner, would
/// otherwise refuse them. The environment is used rather than a config
/// file so the test's repository stays as a real one would be.
pub const GIT_IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "ris"),
    ("GIT_AUTHOR_EMAIL", "ris@localhost"),
    ("GIT_COMMITTER_NAME", "ris"),
    ("GIT_COMMITTER_EMAIL", "ris@localhost"),
];

/// The committed schema of the `--json` contract, with every object
/// closed. The schema leaves objects open, so that a client holding an
/// older copy accepts a newer `ris` of the same format; here the copy
/// is this build's own, so a key the schema does not name is a key
/// some output has and no type declares, and validation fails on it.
fn closed_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/json.schema.json");
        let text = std::fs::read_to_string(path).expect("the committed schema");
        let mut schema: Value = serde_json::from_str(&text).expect("the schema is JSON");
        close(&mut schema);
        schema
    })
}

/// Give every object schema with named properties and no word on
/// others `additionalProperties: false`.
fn close(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if map.get("properties").is_some_and(Value::is_object)
                && !map.contains_key("additionalProperties")
            {
                map.insert("additionalProperties".to_owned(), json!(false));
            }
            map.values_mut().for_each(close);
        }
        Value::Array(items) => items.iter_mut().for_each(close),
        _ => {}
    }
}

/// The command a list of arguments runs, as the schema names it: the
/// subcommand path, such as `phase open`. `None` when the schema has no
/// such command.
pub fn command_of(args: &[&str]) -> Option<String> {
    let defs = closed_schema()["$defs"].as_object()?;
    let words: Vec<&str> = args
        .iter()
        .copied()
        .filter(|a| *a != "--json")
        .take_while(|a| !a.starts_with('-'))
        .collect();
    let known = |name: &str| defs.contains_key(&format!("data.{}", name.replace(' ', "_")));
    match words.as_slice() {
        [a, b, ..] if known(&format!("{a} {b}")) => Some(format!("{a} {b}")),
        [a, ..] if known(a) => Some((*a).to_owned()),
        _ => None,
    }
}

fn errors(schema: &Value, instance: &Value) -> Vec<String> {
    let validator = jsonschema::validator_for(schema).expect("the schema compiles");
    validator
        .iter_errors(instance)
        .map(|e| format!("at {}: {e}", e.instance_path()))
        .collect()
}

/// Check one envelope that `ris --json <args>` printed against the
/// schema: the envelope against the document, and a success's `data`
/// against the data of the command the arguments name. Panics, naming
/// what does not fit, when it does not.
pub fn validate(args: &[&str], envelope: &Value) {
    let schema = closed_schema();
    let said = errors(schema, envelope);
    assert!(
        said.is_empty(),
        "ris {}: not an envelope of the schema:\n  {}\n{envelope:#}",
        args.join(" "),
        said.join("\n  ")
    );
    if envelope["ok"] != true {
        return;
    }
    let command = command_of(args)
        .unwrap_or_else(|| panic!("ris {}: no command of the schema", args.join(" ")));
    let data = json!({
        "$schema": schema["$schema"],
        "$ref": format!("#/$defs/data.{}", command.replace(' ', "_")),
        "$defs": schema["$defs"],
    });
    let said = errors(&data, &envelope["data"]);
    assert!(
        said.is_empty(),
        "ris {}: data is not data.{command} of the schema:\n  {}\n{:#}",
        args.join(" "),
        said.join("\n  "),
        envelope["data"]
    );
}

/// [`validate`] for everything `ris --json <args>` wrote to stdout: one
/// document, or for a stream one envelope per line.
pub fn validate_output(args: &[&str], stdout: &str) {
    match serde_json::from_str::<Value>(stdout) {
        Ok(envelope) => validate(args, &envelope),
        Err(_) => {
            for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
                let envelope: Value = serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("ris {}: {e}: {line}", args.join(" ")));
                validate(args, &envelope);
            }
        }
    }
}
