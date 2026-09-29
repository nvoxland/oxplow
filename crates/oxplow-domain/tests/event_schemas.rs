//! The golden-schema discipline for event payloads
//! (`crates/oxplow-domain/src/events/schema.rs`).
//!
//! - Every registered core `type@v` has a golden at
//!   `schemas/events/<type>@<v>.json`, and the golden equals the schema
//!   generated from the Rust type. A difference means a published
//!   contract changed: add `V + 1` with an `upcast` instead, or — only
//!   for a type that has never shipped — bless the new golden with
//!   `OXPLOW_BLESS=1 cargo test -p oxplow-domain --test event_schemas`.
//! - Every golden file corresponds to a registered type (no orphans).
//! - Every fixture payload at `tests/fixtures/events/<type>@<v>.json`
//!   validates at its own version and, upcast through the chain,
//!   at the newest version (backward-transitive).

use std::fs;
use std::path::PathBuf;

use oxplow_domain::events::schema::EventSchemaRegistry;
use serde_json::Value;

fn dir(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn golden_path(event_type: &str, v: u32) -> PathBuf {
    dir("schemas/events").join(format!("{event_type}@{v}.json"))
}

/// The golden's text form: keys sorted at every level, so the file is the
/// same whichever build produced it (a dependency enabling
/// `serde_json/preserve_order` must not turn into a spurious diff).
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
    // `serde_json::Map` keeps insertion order only under `preserve_order`;
    // without it, it sorts anyway. Either way the output is sorted.
    let mut text = serde_json::to_string_pretty(&sorted(value)).expect("schema serializes");
    text.push('\n');
    text
}

#[test]
fn every_core_type_has_a_golden_schema_that_matches_its_rust_type() {
    let registry = EventSchemaRegistry::core();
    let bless = std::env::var_os("OXPLOW_BLESS").is_some();
    let mut drifted = Vec::new();
    for (event_type, v) in registry.versions() {
        let generated = registry.schema(&event_type, v).unwrap();
        let path = golden_path(&event_type, v);
        let pretty = canonical_pretty(generated);
        match fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == pretty => {}
            Ok(_) if bless => fs::write(&path, &pretty).unwrap(),
            Ok(on_disk) => {
                let golden: Value = serde_json::from_str(&on_disk).unwrap();
                if &golden != generated {
                    drifted.push(format!(
                        "{event_type}@{v}: the Rust payload no longer matches {}. A published \
                         event shape is a contract — add v{} with an `upcast` instead of \
                         changing v{v}.",
                        path.display(),
                        v + 1
                    ));
                } else {
                    drifted.push(format!(
                        "{event_type}@{v}: {} is not in canonical formatting; rewrite it with \
                         OXPLOW_BLESS=1",
                        path.display()
                    ));
                }
            }
            Err(_) if bless => {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, &pretty).unwrap();
            }
            Err(_) => drifted.push(format!(
                "{event_type}@{v}: no golden at {}. For a type that has never shipped, bless \
                 it with OXPLOW_BLESS=1.",
                path.display()
            )),
        }
    }
    assert!(drifted.is_empty(), "\n{}", drifted.join("\n"));
}

#[test]
fn every_golden_schema_is_a_registered_type() {
    let registry = EventSchemaRegistry::core();
    let mut orphans = Vec::new();
    for entry in fs::read_dir(dir("schemas/events")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let Some((event_type, v)) = stem.rsplit_once('@') else {
            orphans.push(format!("{name}: not `<type>@<v>.json`"));
            continue;
        };
        let v: u32 = v.parse().unwrap_or(0);
        if !registry.is_registered(event_type, v) {
            orphans.push(format!("{name}: `{event_type}@{v}` is not registered"));
        }
    }
    assert!(orphans.is_empty(), "\n{}", orphans.join("\n"));
}

#[test]
fn fixtures_validate_at_their_version_and_upcast_to_the_newest() {
    let registry = EventSchemaRegistry::core();
    let mut seen = std::collections::HashSet::new();
    let mut failures = Vec::new();
    for entry in fs::read_dir(dir("tests/fixtures/events")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let (event_type, v) = stem.rsplit_once('@').expect("<type>@<v>.json");
        let v: u32 = v.parse().unwrap();
        let payload: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        seen.insert((event_type.to_string(), v));
        if let Err(e) = registry.validate(event_type, v, &payload) {
            failures.push(format!("{name}: {e}"));
        }
        if let Err(e) = registry.upcast_to_latest(event_type, v, payload) {
            failures.push(format!("{name} → newest: {e}"));
        }
    }
    // Every registered version has at least one fixture, so the upcast
    // chain is exercised from every shipped shape.
    for (event_type, v) in registry.versions() {
        if !seen.contains(&(event_type.clone(), v)) {
            failures.push(format!(
                "{event_type}@{v}: no fixture at tests/fixtures/events/{event_type}@{v}.json"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
