//! Validated `ai_profiles:` declarations. A profile is a named model need
//! an extension declares: the person binds `<extension>/<id>` to a model,
//! else the fallback role's model is used. Declaring one binds nothing.

use oxplow_ai::capabilities::ModelFeature;
use oxplow_ai::config::BuiltinRole;
use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

const MAX_PROFILES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 4096;

/// A named model need of one extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AiProfileDecl {
    pub id: String,
    pub description: String,
    /// The built-in role whose model serves it until the person binds one.
    pub fallback: BuiltinRole,
    /// What the bound model must support.
    pub features: Vec<ModelFeature>,
}

impl AiProfileDecl {
    /// The profile as the AI service resolves it, named in `namespace`.
    pub fn profile(&self, namespace: &str) -> oxplow_ai::config::AiProfile {
        oxplow_ai::config::AiProfile {
            namespace: namespace.to_string(),
            name: self.id.clone(),
            description: self.description.clone(),
            fallback: self.fallback,
            features: self.features.clone(),
        }
    }
}

/// Every AI profile `extensions` declare (a disabled one declares none).
pub fn profiles_of(extensions: &[super::Extension]) -> Vec<oxplow_ai::config::AiProfile> {
    extensions
        .iter()
        .flat_map(|ext| ext.ai_profiles.iter().map(|p| p.profile(&ext.namespace)))
        .collect()
}

/// The built-in roles a profile may fall back to.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum FallbackFile {
    Main,
    Summarize,
    Decide,
}

/// What a bound model must support.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FeatureFile {
    Text,
    ToolCalling,
    Streaming,
    Image,
}

/// The on-disk shape, with unknown fields refused by serde.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiProfileFile {
    /// `[a-z][a-z0-9_]*`, unique in the extension.
    id: String,
    /// What the profile is for (at most 4096 bytes).
    description: String,
    /// The built-in role whose model serves it until a model is bound.
    fallback: FallbackFile,
    /// What the bound model must support.
    #[serde(default)]
    features: Vec<FeatureFile>,
}

fn valid_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `ai_profiles:`, each problem at its `file:line`.
pub fn parse_ai_profiles(
    value: &Value,
    file: &str,
    manifest: &str,
) -> (Vec<AiProfileDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "ai_profiles");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "ai_profile.not_a_list",
                file,
                block,
                "`ai_profiles` must be a list",
            )],
        );
    };
    if items.len() > MAX_PROFILES {
        return (
            Vec::new(),
            vec![at(
                "ai_profile.too_many",
                file,
                block,
                "`ai_profiles` may declare at most 128 profiles",
            )],
        );
    }
    let lines = item_lines(manifest, "ai_profiles");
    let mut profiles: Vec<AiProfileDecl> = Vec::with_capacity(items.len());
    let mut errors = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let line = lines.get(index).copied().or(block);
        let raw = match serde_yaml::from_value::<AiProfileFile>(item.clone()) {
            Ok(raw) => raw,
            Err(e) => {
                errors.push(at(
                    "ai_profile.invalid_fields",
                    file,
                    line,
                    fields_error(&e.to_string()),
                ));
                continue;
            }
        };
        if raw.id.len() > 256 || !valid_id(&raw.id) {
            errors.push(at(
                "ai_profile.invalid_id",
                file,
                line,
                format!(
                    "ai profile id `{}` must be lowercase letters, digits and `_`, starting with a letter",
                    raw.id
                ),
            ));
        } else if raw.description.len() > MAX_DESCRIPTION_BYTES {
            errors.push(at(
                "ai_profile.description_too_long",
                file,
                line,
                "ai profile description exceeds 4096 bytes",
            ));
        } else if profiles.iter().any(|p| p.id == raw.id) {
            errors.push(at(
                "ai_profile.duplicate_id",
                file,
                line,
                format!("ai profile id `{}` is declared twice", raw.id),
            ));
        } else {
            let mut features = Vec::new();
            for feature in &raw.features {
                let feature = match feature {
                    FeatureFile::Text => ModelFeature::Text,
                    FeatureFile::ToolCalling => ModelFeature::ToolCalling,
                    FeatureFile::Streaming => ModelFeature::Streaming,
                    FeatureFile::Image => ModelFeature::Image,
                };
                if !features.contains(&feature) {
                    features.push(feature);
                }
            }
            profiles.push(AiProfileDecl {
                id: raw.id,
                description: raw.description,
                fallback: match raw.fallback {
                    FallbackFile::Main => BuiltinRole::Main,
                    FallbackFile::Summarize => BuiltinRole::Summarize,
                    FallbackFile::Decide => BuiltinRole::Decide,
                },
                features,
            });
        }
    }
    (profiles, errors)
}

/// A serde failure as the author reads it: the wrong `fallback` and
/// `features` values say what is allowed.
fn fields_error(serde: &str) -> String {
    if serde.contains("unknown variant") && serde.contains("main") {
        "ai profile `fallback` is `main`, `summarize` or `decide`".into()
    } else if serde.contains("unknown variant") {
        "ai profile `features` are `text`, `tool_calling`, `streaming` or `image`".into()
    } else if serde.contains("missing field `fallback`") {
        "ai profile needs a `fallback`: `main`, `summarize` or `decide`".into()
    } else {
        format!("ai profile declaration has invalid fields: {serde}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "ai_profiles:\n  - id: review\n    description: Reviews.\n    fallback: summarize\n    features: [tool_calling]\n  - id: bad-id\n    description: x\n    fallback: main\n";

    fn parse(manifest: &str) -> (Vec<AiProfileDecl>, Vec<Diagnostic>) {
        let doc: Value = serde_yaml::from_str(manifest).unwrap();
        parse_ai_profiles(&doc["ai_profiles"], "extension.yaml", manifest)
    }

    #[test]
    fn a_valid_profile_parses_and_each_problem_has_its_line() {
        let (profiles, errors) = parse(MANIFEST);
        assert_eq!(
            profiles,
            [AiProfileDecl {
                id: "review".into(),
                description: "Reviews.".into(),
                fallback: BuiltinRole::Summarize,
                features: vec![ModelFeature::ToolCalling],
            }]
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .render("")
                .starts_with("extension.yaml:6: ai profile id `bad-id`"),
            "{errors:?}"
        );
    }

    #[test]
    fn the_other_mistakes_are_named() {
        let cases = [
            (
                "ai_profiles:\n  - { id: a, description: x, fallback: big }\n",
                "`fallback` is `main`, `summarize` or `decide`",
            ),
            (
                "ai_profiles:\n  - { id: a, description: x, fallback: main, features: [telepathy] }\n",
                "`features` are `text`",
            ),
            (
                "ai_profiles:\n  - { id: a, description: x }\n",
                "needs a `fallback`",
            ),
            (
                "ai_profiles:\n  - { id: a, description: x, fallback: main }\n  - { id: a, description: y, fallback: main }\n",
                "`a` is declared twice",
            ),
        ];
        for (manifest, says) in cases {
            let (_, errors) = parse(manifest);
            assert!(
                errors
                    .iter()
                    .any(|e| e.message.contains(says) && e.render("").contains("extension.yaml:")),
                "{manifest}: {errors:?}"
            );
        }
        let long = format!(
            "ai_profiles:\n  - {{ id: a, description: \"{}\", fallback: main }}\n",
            "x".repeat(MAX_DESCRIPTION_BYTES + 1)
        );
        assert!(parse(&long).1[0].message.contains("exceeds 4096 bytes"));
    }
}
