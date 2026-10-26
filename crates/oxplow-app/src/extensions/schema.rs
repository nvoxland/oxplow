//! JSON Schemas for an extension's files: `extension.yaml` (from
//! [`ManifestV2`]), generated from the serde types the loader reads, so an editor
//! (yaml-language-server) completes and checks what the loader accepts.
//! Checked in at `docs/reference/schemas/`; a test keeps them equal to
//! what this generates (`OXPLOW_BLESS=1` rewrites them).
//!
//! A schema holds the shape — the keys, their types, which are required.
//! What the loader checks beyond it (an id's spelling, a page that
//! exists, which trigger keys go together) stays the loader's to say.

use schemars::generate::SchemaSettings;
use serde_json::Value;

use super::manifest_v2::ManifestV2;

/// Where the published manifest schema lives (the docs site).
pub const MANIFEST_SCHEMA_URL: &str =
    "https://nvoxland.github.io/oxplow/reference/schemas/extension.schema.json";

/// The first line of a scaffolded file: points yaml-language-server at
/// its schema.
pub fn modeline(url: &str) -> String {
    format!("# yaml-language-server: $schema={url}\n")
}

/// Draft-07: the dialect editors' YAML tooling reads most widely.
fn generate<T: schemars::JsonSchema>(id: &str, title: &str) -> Value {
    let schema = SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut value = serde_json::to_value(schema).expect("a schema serializes");
    if let Some(map) = value.as_object_mut() {
        map.insert("$id".into(), Value::String(id.into()));
        map.insert("title".into(), Value::String(title.into()));
    }
    value
}

/// The schema of `extension.yaml` (manifest version 2).
pub fn manifest_schema() -> Value {
    generate::<ManifestV2>(MANIFEST_SCHEMA_URL, "oxplow extension.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The checked-in schema at `rel` is `generated`; `OXPLOW_BLESS=1`
    /// writes it.
    fn golden(rel: &str, generated: &Value) {
        let path = repo_root().join(rel);
        let text = serde_json::to_string_pretty(generated).expect("serializes") + "\n";
        if std::env::var_os("OXPLOW_BLESS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            return;
        }
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            on_disk == text,
            "{rel} is stale: regenerate it with OXPLOW_BLESS=1"
        );
    }

    #[test]
    fn the_checked_in_manifest_schema_is_the_generated_one() {
        golden(
            "docs/reference/schemas/extension.schema.json",
            &manifest_schema(),
        );
    }

    fn yaml_json(text: &str) -> Value {
        let yaml: serde_yaml::Value = serde_yaml::from_str(text).expect("YAML parses");
        serde_json::to_value(yaml).expect("YAML is JSON")
    }

    fn problems(schema: &Value, text: &str) -> Vec<String> {
        let validator = jsonschema::validator_for(schema).expect("the schema compiles");
        validator
            .iter_errors(&yaml_json(text))
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect()
    }

    /// Every bundled extension's manifest — and the example extensions'
    /// — validates against the schema an editor checks it with.
    #[test]
    fn every_bundled_and_example_manifest_validates() {
        let manifest = manifest_schema();
        let mut checked = 0;
        for ext in crate::bundled_extensions::BUNDLED {
            let text = ext.file("extension.yaml").expect("a manifest is text");
            let p = problems(&manifest, text);
            assert!(p.is_empty(), "{}/extension.yaml: {p:#?}", ext.name);
            checked += 1;
        }
        assert!(checked >= 2, "the bundled manifests were found");
        let examples = repo_root().join("examples/extensions");
        for dir in std::fs::read_dir(&examples).expect("examples/extensions") {
            let dir = dir.unwrap().path();
            if !dir.is_dir() {
                continue; // the examples' README
            }
            let text = std::fs::read_to_string(dir.join("extension.yaml")).unwrap();
            let p = problems(&manifest, &text);
            assert!(p.is_empty(), "{}: {p:#?}", dir.display());
        }
    }

    const OK: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Count things\n  examples:\n    - { name: one, input: {}, expect: {} }\n";

    /// What the loader refuses as an unknown key, the schema does too —
    /// at the top level and inside a kind's entries.
    #[test]
    fn unknown_keys_do_not_validate() {
        let schema = manifest_schema();
        assert!(problems(&schema, OK).is_empty());
        assert!(!problems(&schema, &format!("{OK}gauges: []\n")).is_empty());
        let effect = "effects:\n  - { id: on-done, summary: s, on: [work_item.state_changed], entry: e.star }\n";
        assert!(
            problems(&schema, &format!("{OK}{effect}")).is_empty(),
            "{:?}",
            problems(&schema, &format!("{OK}{effect}"))
        );
        let stray = "effects:\n  - { id: on-done, summary: s, on: [work_item.state_changed], entry: e.star, when: now }\n";
        assert!(!problems(&schema, &format!("{OK}{stray}")).is_empty());
    }

    /// The scaffolds point an editor at the schemas.
    #[test]
    fn modelines_name_the_published_schemas() {
        assert_eq!(
            modeline(MANIFEST_SCHEMA_URL),
            "# yaml-language-server: $schema=https://nvoxland.github.io/oxplow/reference/schemas/extension.schema.json\n"
        );
        assert_eq!(
            manifest_schema()["$id"],
            Value::String(MANIFEST_SCHEMA_URL.into())
        );
    }
}
