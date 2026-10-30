//! Offline, value-free schema diagnostics for extension configuration.
//!
//! Defaults are object fragments: direct `required` and `minProperties`
//! failures on mergeable objects defer to final validation. Arrays replace as
//! complete values, so objects inside arrays must satisfy those constraints.
//! Other constraints (including branch selection) apply to defaults as written.
//! This does not construct hypothetical completions of conditional schemas.

use jsonschema::error::ValidationErrorKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_ISSUES: usize = 32;

/// A versioned declaration of non-secret settings. This is data, not consent
/// or a credential reference. Stored overrides and host services are separate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationFile {
    #[schemars(range(min = 1))]
    pub version: u32,
    #[specta(type = oxplow_domain::Json)]
    pub schema: Value,
    #[serde(default = "empty_object")]
    #[specta(type = oxplow_domain::Json)]
    pub defaults: Value,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

impl ConfigurationFile {
    /// Resolve explicit project/personal fragments with this declaration's
    /// schema. This pure host adapter exposes no credentials or execution rights.
    pub fn resolve(
        &self,
        extension: &str,
        project: Option<&Value>,
        personal: Option<&Value>,
    ) -> Result<super::config::ResolvedConfig, Vec<super::config::ConfigIssue>> {
        use super::config::{ConfigIssue, ConfigLayer, ConfigViolation};
        if !oxplow_domain::contribution::valid_extension_name(extension) {
            return Err(vec![ConfigIssue {
                extension: String::new(),
                layer: ConfigLayer::Defaults,
                pointer: String::new(),
                code: "invalid_namespace".into(),
            }]);
        }
        let declaration_error = |issue: SchemaViolation| ConfigIssue {
            extension: extension.to_owned(),
            layer: ConfigLayer::Defaults,
            pointer: issue.pointer,
            code: issue.code,
        };
        if self.version == 0 {
            return Err(vec![declaration_error(violation("invalid_version"))]);
        }
        let schema =
            ConfigSchema::compile(&self.schema).map_err(|issue| vec![declaration_error(issue)])?;
        super::config::resolve_extension_config(
            extension,
            &self.defaults,
            project,
            personal,
            |layer, value| {
                let violations = match layer {
                    ConfigLayer::Defaults => schema.validate_defaults(value),
                    ConfigLayer::Resolved => schema.validate_resolved(value),
                    // Fragments never need satisfy a final required-field schema.
                    ConfigLayer::Project | ConfigLayer::Personal => {
                        vec![violation("invalid_validation_layer")]
                    }
                };
                violations
                    .into_iter()
                    .map(|issue| ConfigViolation {
                        pointer: issue.pointer,
                        code: issue.code,
                    })
                    .collect()
            },
        )
    }

    /// Prepare this declaration without requiring user-supplied fields yet.
    /// No library error text or invalid input values enter load diagnostics.
    pub fn prepare(&self) -> Result<ConfigSchema, Vec<SchemaViolation>> {
        if self.version == 0 {
            return Err(vec![violation("invalid_version")]);
        }
        let schema = ConfigSchema::compile(&self.schema).map_err(|issue| vec![issue])?;
        let issues = schema.validate_defaults(&self.defaults);
        if issues.is_empty() {
            Ok(schema)
        } else {
            Err(issues)
        }
    }
}

/// Safe metadata only. Never holds an instance value or formatted library error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaViolation {
    pub pointer: String,
    pub code: String,
}

/// One schema shared by defaults and resolved-value validation. Compilation is
/// explicitly offline, regardless of dependency feature unification.
pub struct ConfigSchema {
    validator: jsonschema::Validator,
}

impl ConfigSchema {
    /// The root declares an object. Compilation errors deliberately discard
    /// the library error: it may include schema literals or a retrieval URI.
    pub fn compile(schema: &Value) -> Result<Self, SchemaViolation> {
        bounded_json(schema)?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(violation("object_schema_required"));
        }
        let validator = jsonschema::options()
            .offline()
            .build(schema)
            .map_err(|_| violation("invalid_schema"))?;
        Ok(Self { validator })
    }

    /// Validate the supplied defaults. Missing object fields can be provided by
    /// a higher layer. Every present value and complete array is checked.
    pub fn validate_defaults(&self, defaults: &Value) -> Vec<SchemaViolation> {
        self.validate(defaults, true)
    }

    /// Validate all final constraints, including every required field. No
    /// incomplete or partially validated configuration is returned by this API.
    pub fn validate_resolved(&self, resolved: &Value) -> Vec<SchemaViolation> {
        self.validate(resolved, false)
    }

    fn validate(&self, value: &Value, partial: bool) -> Vec<SchemaViolation> {
        if let Err(issue) = bounded_json(value) {
            return vec![issue];
        }
        if !value.is_object() {
            return vec![violation("object_required")];
        }
        let mut issues = Vec::new();
        for error in self.validator.iter_errors(value) {
            let pointer = error.instance_path().to_string();
            if partial
                && matches!(
                    error.kind(),
                    ValidationErrorKind::Required { .. }
                        | ValidationErrorKind::MinProperties { .. }
                )
                && mergeable_object(value, &pointer)
            {
                continue;
            }
            if issues.len() == MAX_ISSUES {
                issues.push(violation("additional_violations"));
                break;
            }
            issues.push(SchemaViolation {
                pointer,
                // Only built-in keywords are registered. Do not format errors,
                // their instances, enum choices, or unexpected property lists.
                code: error.kind().keyword().to_owned(),
            });
        }
        issues.sort_by(|a, b| (&a.pointer, &a.code).cmp(&(&b.pointer, &b.code)));
        issues.dedup();
        issues
    }
}

/// Objects remain mergeable only until an array boundary. JSON pointer segments
/// are decoded individually, so keys containing '/' or '~' keep their meaning.
fn mergeable_object(root: &Value, pointer: &str) -> bool {
    let mut node = root;
    if !pointer.is_empty() {
        let Some(path) = pointer.strip_prefix('/') else {
            return false;
        };
        for segment in path.split('/') {
            let Some(object) = node.as_object() else {
                return false;
            };
            let key = segment.replace("~1", "/").replace("~0", "~");
            let Some(child) = object.get(&key) else {
                return false;
            };
            node = child;
        }
    }
    node.is_object()
}

fn violation(code: &str) -> SchemaViolation {
    SchemaViolation {
        pointer: String::new(),
        code: code.to_owned(),
    }
}

fn bounded_json(value: &Value) -> Result<(), SchemaViolation> {
    super::config::check_json_limits(value).map_err(violation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declaration() -> ConfigurationFile {
        serde_json::from_value(json!({
            "version": 1,
            "schema": {
                "type": "object",
                "required": ["limit"],
                "additionalProperties": false,
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "mode": {"enum": ["fast", "safe-hunter2"]}
                }
            },
            "defaults": {"limit": 10}
        }))
        .unwrap()
    }

    #[test]
    fn a_declaration_resolves_layers_against_its_schema() {
        let declaration = declaration();
        assert!(declaration.prepare().is_ok());
        let resolved = declaration
            .resolve(
                "alpha",
                Some(&json!({"limit": 20})),
                Some(&json!({"mode": "fast"})),
            )
            .unwrap();
        assert_eq!(resolved.values, json!({"limit": 20, "mode": "fast"}));
    }

    #[test]
    fn a_value_failing_the_schema_yields_value_free_issues() {
        let declaration = declaration();
        for (project, personal) in [
            (json!({"limit": "hunter2"}), json!({})),
            (json!({"limit": 1000}), json!({})),
            (json!({}), json!({"mode": "nope-hunter2"})),
            (json!({"unexpected-hunter2": 1}), json!({})),
        ] {
            let issues = declaration
                .resolve("alpha", Some(&project), Some(&personal))
                .expect_err("the schema refuses it");
            assert!(!issues.is_empty());
            let text = format!("{issues:?}");
            assert!(
                !text.contains("hunter2") && !text.contains("1000"),
                "{text}"
            );
            assert!(issues.iter().all(|issue| !issue.code.is_empty()));
        }
    }

    #[test]
    fn required_fields_are_checked_on_the_resolved_object() {
        let mut declaration = declaration();
        declaration.defaults = json!({});
        // Defaults alone are a fragment: fine to declare, not to resolve.
        assert!(declaration.prepare().is_ok());
        assert!(declaration.resolve("alpha", None, None).is_err());
        assert!(declaration
            .resolve("alpha", Some(&json!({"limit": 5})), None)
            .is_ok());
    }

    #[test]
    fn a_bad_declaration_is_refused_when_prepared() {
        let mut declaration = declaration();
        declaration.version = 0;
        assert!(declaration.prepare().is_err());

        let mut declaration = self::declaration();
        declaration.schema = json!({"type": "string"});
        assert!(declaration.prepare().is_err());

        let mut declaration = self::declaration();
        declaration.defaults = json!({"limit": 1000});
        assert!(declaration.prepare().is_err());
    }

    #[test]
    fn a_schema_cannot_fetch_anything() {
        let mut declaration = declaration();
        declaration.schema = json!({
            "type": "object",
            "properties": {"x": {"$ref": "https://example.invalid/schema.json"}}
        });
        let issues = declaration
            .prepare()
            .err()
            .expect("an external ref is refused");
        assert!(!format!("{issues:?}").contains("example.invalid"));
    }
}
