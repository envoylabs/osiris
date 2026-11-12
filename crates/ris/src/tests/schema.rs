use super::*;

#[test]
fn every_command_has_one_entry_and_every_reference_resolves() {
    let mut g = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    let names: Vec<&str> = commands(&mut g).into_iter().map(|(n, _)| n).collect();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "a command is listed twice");

    let doc = document();
    let defs = doc["$defs"].as_object().unwrap();
    for name in &names {
        assert!(defs.contains_key(&data_name(name)), "{name}");
    }
    // Every $ref in the document points at a definition in it.
    fn refs(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(r)) = m.get("$ref") {
                    out.push(r.clone());
                }
                m.values().for_each(|v| refs(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    refs(&doc, &mut found);
    assert!(!found.is_empty());
    for r in found {
        let name = r.strip_prefix("#/$defs/").unwrap_or_else(|| panic!("{r}"));
        assert!(defs.contains_key(name), "{r} does not resolve");
    }
}

#[test]
fn the_document_is_the_same_each_time_it_is_generated() {
    assert_eq!(text(), text());
}

/// The committed file is what the types generate: a change to any
/// output type that is not regenerated and committed fails here.
#[test]
fn the_committed_schema_is_the_generated_one() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(PATH);
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == text(),
        "{PATH} is not what the output types generate\n  \
             next: `cargo run -p ris -- schema > {PATH}`, read the diff, and commit it; \
             a key removed, renamed or reshaped there is a new format, not format {JSON_FORMAT}"
    );
}
