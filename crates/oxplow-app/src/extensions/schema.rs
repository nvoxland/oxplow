//! JSON Schemas for an extension's files: `extension.yaml` (from
//! [`ManifestV2`]) and `lenses/*.yaml` (from the lens file's on-disk
//! struct), generated from the serde types the loader reads, so an editor
//! (yaml-language-server) completes and checks what the loader accepts.
//! Checked in at `docs/reference/schemas/`; a test keeps them equal to
//! what this generates (`OXPLOW_BLESS=1` rewrites them).
//!
//! A schema holds the shape — the keys, their types, which are required.
//! What the loader checks beyond it (an id's spelling, a lens that
//! exists, which trigger keys go together) stays the loader's to say.

use schemars::generate::SchemaSettings;
use serde_json::Value;

use super::manifest_v2::ManifestV2;
use super::LensFile;

/// Where the published manifest schema lives (the docs site).
pub const MANIFEST_SCHEMA_URL: &str =
    "https://nvoxland.github.io/oxplow/reference/schemas/extension.schema.json";
/// Where the published lens schema lives.
pub const LENS_SCHEMA_URL: &str =
    "https://nvoxland.github.io/oxplow/reference/schemas/lens.schema.json";

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

/// The schema of a lens file (`lenses/<slug>.yaml`).
pub fn lens_schema() -> Value {
    generate::<LensFile>(LENS_SCHEMA_URL, "oxplow lens")
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

    #[test]
    fn the_checked_in_lens_schema_is_the_generated_one() {
        golden("docs/reference/schemas/lens.schema.json", &lens_schema());
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

    /// Every bundled extension's manifest and lenses — and the example
    /// extensions' — validate against the schemas an editor checks them
    /// with.
    #[test]
    fn every_bundled_manifest_and_lens_validates() {
        let manifest = manifest_schema();
        let lens = lens_schema();
        let mut checked = 0;
        for ext in crate::bundled_extensions::BUNDLED {
            for (path, text) in ext.files {
                let schema = if *path == "extension.yaml" {
                    &manifest
                } else if path.starts_with("lenses/") && path.ends_with(".yaml") {
                    &lens
                } else {
                    continue;
                };
                let p = problems(schema, text);
                assert!(p.is_empty(), "{}/{path}: {p:#?}", ext.name);
                checked += 1;
            }
        }
        assert!(checked > 2, "the bundled files were found");
        let examples = repo_root().join("examples/extensions");
        for dir in std::fs::read_dir(&examples).expect("examples/extensions") {
            let dir = dir.unwrap().path();
            let text = std::fs::read_to_string(dir.join("extension.yaml")).unwrap();
            let p = problems(&manifest, &text);
            assert!(p.is_empty(), "{}: {p:#?}", dir.display());
            for f in std::fs::read_dir(dir.join("lenses")).into_iter().flatten() {
                let f = f.unwrap().path();
                let p = problems(&lens, &std::fs::read_to_string(&f).unwrap());
                assert!(p.is_empty(), "{}: {p:#?}", f.display());
            }
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
        let lens = lens_schema();
        assert!(problems(&lens, "title: T\nquery: SELECT 1\n").is_empty());
        assert!(!problems(&lens, "title: T\nquery: SELECT 1\nsize: big\n").is_empty());
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
        assert_eq!(lens_schema()["$id"], Value::String(LENS_SCHEMA_URL.into()));
    }
}
