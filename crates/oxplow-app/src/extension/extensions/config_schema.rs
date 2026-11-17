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

/// A versioned declaration of settings. This is data, not consent or a
/// credential reference. Stored overrides and host services are separate.
/// A field's default is the `default` keyword on its schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationFile {
    #[schemars(range(min = 1))]
    pub version: u32,
    #[specta(type = oxplow_domain::Json)]
    pub schema: Value,
}

/// How deep a schema's references and nested objects are followed for
/// defaults. Deeper declarations are refused elsewhere.
const MAX_DEFAULT_DEPTH: usize = 32;

impl ConfigurationFile {
    /// The defaults tree: each field's `default` keyword (a local `$ref`'s
    /// target supplies one the reference does not state), nested objects
    /// collecting their fields' defaults. A field without one is absent.
    pub fn defaults(&self) -> Value {
        default_of(&self.schema, &self.schema, 0)
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()))
    }
}

fn default_of(root: &Value, node: &Value, depth: usize) -> Option<Value> {
    if depth > MAX_DEFAULT_DEPTH {
        return None;
    }
    if let Some(default) = node.get("default") {
        return Some(default.clone());
    }
    if let Some(target) = node
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| root.pointer(pointer))
    {
        return default_of(root, target, depth + 1);
    }
    let properties = node.get("properties")?.as_object()?;
    let fields: serde_json::Map<_, _> = properties
        .iter()
        .filter_map(|(key, field)| default_of(root, field, depth + 1).map(|d| (key.clone(), d)))
        .collect();
    (!fields.is_empty()).then_some(Value::Object(fields))
}

/// `when: { config: <pointer>, in: [<values>] }`: a contribution that is
/// there only while a setting of its extension's own configuration is one
/// of `in`. The one condition type every contribution kind that is offered
/// by a setting shares.
#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ConfigCondition {
    /// A JSON pointer into the extension's own configuration, naming a
    /// scalar key of `configuration.schema` (`/tasks/workflow`).
    pub config: String,
    /// The values for which it holds: of the key's type, and of its `enum`
    /// when it has one.
    #[serde(rename = "in")]
    #[specta(type = Vec<oxplow_domain::Json>)]
    pub values: Vec<Value>,
}

impl ConfigCondition {
    /// Whether it holds over `effective`, the extension's effective
    /// configuration values.
    pub fn holds(&self, effective: &Value) -> bool {
        effective
            .pointer(&self.config)
            .is_some_and(|value| self.values.contains(value))
    }

    /// Why it can never hold against `declaration` (the extension's own):
    /// `(code, message)`, or `None` when it can.
    pub fn problem(
        &self,
        declaration: Option<&ConfigurationFile>,
    ) -> Option<(&'static str, String)> {
        let pointer = &self.config;
        let Some(declaration) = declaration else {
            return Some((
                "when.undeclared",
                format!(
                    "`when` reads `{pointer}` of the extension's own configuration, which it \
                     doesn't declare (`configuration:` in extension.yaml)"
                ),
            ));
        };
        let Some(key) = schema_key(&declaration.schema, pointer) else {
            return Some((
                "when.unknown_key",
                format!("`when` names `{pointer}`, no key of `configuration.schema`"),
            ));
        };
        let kind = key.get("type").and_then(Value::as_str);
        let Some(kind @ ("integer" | "number" | "string" | "boolean")) = kind else {
            return Some((
                "when.not_scalar",
                format!(
                    "`when` names `{pointer}`, which is a `{}` in the configuration schema; \
                     it compares a number, string or boolean",
                    kind.unwrap_or("value of no type")
                ),
            ));
        };
        if self.values.is_empty() {
            return Some((
                "when.empty",
                format!("`when` on `{pointer}` lists no value in `in`, so it never holds"),
            ));
        }
        let of_kind = |value: &Value| match kind {
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "string" => value.is_string(),
            _ => value.is_boolean(),
        };
        if let Some(value) = self.values.iter().find(|v| !of_kind(v)) {
            return Some((
                "when.wrong_type",
                format!("`when` on `{pointer}` lists {value}, which isn't a `{kind}`"),
            ));
        }
        let choices = key.get("enum").and_then(Value::as_array)?;
        let outside: Vec<String> = self
            .values
            .iter()
            .filter(|v| !choices.contains(v))
            .map(Value::to_string)
            .collect();
        (!outside.is_empty()).then(|| {
            (
                "when.not_in_enum",
                format!(
                    "`when` on `{pointer}` lists {}, which its `enum` doesn't allow",
                    outside.join(", ")
                ),
            )
        })
    }
}

/// The schema of the property at JSON pointer `pointer` in `schema`,
/// following `properties`; `None` when the schema declares no such property.
pub(crate) fn schema_key<'a>(schema: &'a Value, pointer: &str) -> Option<&'a Value> {
    let mut node = schema;
    for token in pointer.strip_prefix('/')?.split('/') {
        let token = token.replace("~1", "/").replace("~0", "~");
        node = node.get("properties")?.get(&token)?;
    }
    Some(node)
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
        // Settings are stored under the namespace, which is what names them.
        if !oxplow_domain::contribution::valid_namespace(extension) {
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
            &self.defaults(),
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
        let issues = schema.validate_defaults(&self.defaults());
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

/// A namespace's settings: its extensions' declarations as one, and what
/// was refused, by the index of the declaration it refused.
#[derive(Debug, Default)]
pub struct Merged {
    pub declaration: Option<ConfigurationFile>,
    pub problems: Vec<(usize, String)>,
}

/// The declarations of one namespace's extensions, in load order, as the
/// one document they share: the first's version, every top-level property
/// and default, `required` joined. One at another version, or declaring a
/// key an earlier one does, is refused whole.
pub fn merge(declarations: &[(&str, &ConfigurationFile)]) -> Merged {
    let mut out = Merged::default();
    let mut owners: Vec<(String, String)> = Vec::new();
    for (i, (extension, decl)) in declarations.iter().enumerate() {
        let properties = decl
            .schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(merged) = out.declaration.as_mut() else {
            owners.extend(
                properties
                    .keys()
                    .map(|k| (k.clone(), extension.to_string())),
            );
            out.declaration = Some((*decl).clone());
            continue;
        };
        if decl.version != merged.version {
            let first = &declarations[0].0;
            out.problems.push((
                i,
                format!(
                    "`configuration` is `version: {}`, and `{first}` in its namespace declares \
                     `version: {}`: extensions in one namespace share their settings, at one \
                     version",
                    decl.version, merged.version
                ),
            ));
            continue;
        }
        if let Some((key, owner)) = owners.iter().find(|(k, _)| properties.contains_key(k)) {
            out.problems.push((
                i,
                format!(
                    "`configuration` key `{key}` is also `{owner}`'s, and extensions in one \
                     namespace share their settings"
                ),
            ));
            continue;
        }
        owners.extend(
            properties
                .keys()
                .map(|k| (k.clone(), extension.to_string())),
        );
        let schema = merged.schema.as_object_mut();
        if let Some(schema) = schema {
            let into = schema
                .entry("properties")
                .or_insert_with(|| Value::Object(Default::default()));
            if let Some(into) = into.as_object_mut() {
                into.extend(properties);
            }
            if let Some(required) = decl.schema.get("required").and_then(Value::as_array) {
                let into = schema
                    .entry("required")
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Some(into) = into.as_array_mut() {
                    into.extend(required.iter().cloned());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    use serde_json::json;

    fn decl(version: u32, key: &str, default: i64) -> ConfigurationFile {
        ConfigurationFile {
            version,
            schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { key: { "type": "integer", "default": default } },
            }),
        }
    }

    /// Settings are the namespace's: one document, whichever of its
    /// extensions declares a key. Their declarations merge into one schema
    /// at one version; a second version, or a key two declare, is refused
    /// on the later one, which then has none.
    #[test]
    fn a_namespaces_declarations_merge_into_one() {
        let merged = merge(&[("first", &decl(1, "a", 1)), ("second", &decl(1, "b", 2))]);
        assert_eq!(merged.problems, Vec::<(usize, String)>::new());
        let one = merged.declaration.unwrap();
        assert_eq!(one.version, 1);
        assert_eq!(one.defaults(), json!({ "a": 1, "b": 2 }));
        assert_eq!(
            one.schema["properties"],
            json!({ "a": { "type": "integer", "default": 1 }, "b": { "type": "integer", "default": 2 } })
        );
        assert_eq!(one.schema["additionalProperties"], json!(false));

        let clash = merge(&[
            ("first", &decl(1, "a", 1)),
            ("second", &decl(1, "a", 2)),
            ("third", &decl(2, "c", 3)),
        ]);
        assert_eq!(clash.declaration.unwrap().defaults(), json!({ "a": 1 }));
        let said: Vec<(usize, &str)> = clash
            .problems
            .iter()
            .map(|(i, m)| (*i, m.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                (1, "`configuration` key `a` is also `first`'s, and extensions in one namespace share their settings"),
                (2, "`configuration` is `version: 2`, and `first` in its namespace declares `version: 1`: extensions in one namespace share their settings, at one version"),
            ]
        );
    }
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
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10},
                    "mode": {"enum": ["fast", "safe-hunter2"]}
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn defaults_are_the_schema_fields_defaults() {
        let declaration: ConfigurationFile = serde_json::from_value(json!({
            "version": 1,
            "schema": {
                "type": "object",
                "$defs": {"mode": {"enum": ["fast", "safe"], "default": "safe"}},
                "properties": {
                    "limit": {"type": "integer", "default": 10},
                    "mode": {"$ref": "#/$defs/mode"},
                    "picked": {"$ref": "#/$defs/mode", "default": "fast"},
                    "nested": {"type": "object", "properties": {
                        "a": {"type": "integer", "default": 1},
                        "b": {"type": "integer"}
                    }},
                    "none": {"type": "string"}
                }
            }
        }))
        .unwrap();
        assert_eq!(
            declaration.defaults(),
            json!({"limit": 10, "mode": "safe", "picked": "fast", "nested": {"a": 1}})
        );
        // The old separate tree is not part of a declaration any more.
        assert!(serde_json::from_value::<ConfigurationFile>(json!({
            "version": 1,
            "schema": {"type": "object"},
            "defaults": {}
        }))
        .is_err());
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
        declaration.schema["properties"]["limit"]
            .as_object_mut()
            .unwrap()
            .remove("default");
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
        declaration.schema["properties"]["limit"]["default"] = json!(1000);
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
