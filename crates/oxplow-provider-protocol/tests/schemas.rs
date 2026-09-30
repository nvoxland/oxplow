//! The golden-schema discipline for the provider protocol (the
//! `event_schemas.rs` pattern): every wire type has a golden at
//! `schemas/<name>.json` equal to the schema generated from its Rust type,
//! and every golden names a wire type. A difference is a protocol change:
//! bump `PROTOCOL_VERSION` rather than edit a shipped shape; bless a new
//! one with `OXPLOW_BLESS=1`.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

fn canonical_pretty(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), sorted(&map[k]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let mut text = serde_json::to_string_pretty(&sorted(value)).expect("schema serializes");
    text.push('\n');
    text
}

#[test]
fn every_wire_type_has_a_golden_schema_that_matches_its_rust_type() {
    let bless = std::env::var_os("OXPLOW_BLESS").is_some();
    let mut problems = Vec::new();
    let all = oxplow_provider_protocol::schemas::all();
    for (name, schema) in &all {
        let path = dir().join(format!("{name}.json"));
        let pretty = canonical_pretty(schema);
        match fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == pretty => {}
            _ if bless => {
                fs::create_dir_all(dir()).unwrap();
                fs::write(&path, &pretty).unwrap();
            }
            Ok(_) => problems.push(format!(
                "{name}: the Rust type no longer matches {} — a shipped wire shape is a \
                 contract; bump PROTOCOL_VERSION rather than change it",
                path.display()
            )),
            Err(_) => problems.push(format!(
                "{name}: no golden at {}; bless a new wire type with OXPLOW_BLESS=1",
                path.display()
            )),
        }
    }
    if let Ok(entries) = fs::read_dir(dir()) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let stem = file.trim_end_matches(".json");
            if !all.iter().any(|(name, _)| *name == stem) {
                problems.push(format!("{file}: a golden for no wire type"));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn a_message_validates_against_its_golden() {
    use oxplow_provider_protocol::schemas::{for_message, validate};
    let ok = serde_json::json!({ "protocol_version": "1", "host": { "name": "oxplow", "version": "0.7.0" } });
    assert_eq!(
        validate(for_message("initialize", false).unwrap(), &ok),
        Ok(())
    );
    let bad = serde_json::json!({ "protocol_version": 1, "host": {}, "extra": true });
    let errors = validate("initialize_params", &bad).unwrap_err();
    assert!(errors.len() >= 2, "{errors:?}");
}
