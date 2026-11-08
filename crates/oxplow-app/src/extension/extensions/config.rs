//! Pure layered configuration resolution for one extension.
//!
//! Schema compilation, persistence, secret handling and authorization belong
//! to the host. This module only merges bounded JSON objects and records where
//! each effective leaf came from.

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use specta::Type;

pub const MAX_CONFIG_BYTES: usize = 1_048_576;
pub const MAX_CONFIG_DEPTH: usize = 64;
pub const MAX_CONFIG_ISSUES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConfigLayer {
    Defaults,
    Project,
    Personal,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ConfigViolation {
    pub pointer: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ConfigIssue {
    pub extension: String,
    pub layer: ConfigLayer,
    pub pointer: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ResolvedConfig {
    #[specta(type = oxplow_domain::Json)]
    pub values: serde_json::Value,
    pub origins: BTreeMap<String, ConfigLayer>,
}

/// Resolve defaults, shared project values and local personal overrides.
/// Object fields merge recursively; arrays and every non-object value replace
/// the prior subtree. Validation is supplied by the host so this function
/// stays independent of schema loading and execution authority.
pub fn resolve_extension_config(
    extension: &str,
    defaults: &serde_json::Value,
    project: Option<&serde_json::Value>,
    personal: Option<&serde_json::Value>,
    validate: impl Fn(ConfigLayer, &serde_json::Value) -> Vec<ConfigViolation>,
) -> Result<ResolvedConfig, Vec<ConfigIssue>> {
    if !oxplow_domain::contribution::valid_namespace(extension) {
        return Err(vec![ConfigIssue {
            extension: String::new(),
            layer: ConfigLayer::Defaults,
            pointer: String::new(),
            code: "invalid_namespace".into(),
        }]);
    }

    let mut issues = Vec::new();
    check_layer(
        extension,
        ConfigLayer::Defaults,
        Some(defaults),
        &mut issues,
    );
    check_layer(extension, ConfigLayer::Project, project, &mut issues);
    check_layer(extension, ConfigLayer::Personal, personal, &mut issues);
    if !issues.is_empty() {
        return Err(sort_issues(issues));
    }

    let mut values = defaults.clone();
    let mut origins = BTreeMap::new();
    assign_leaf_origins(&values, ConfigLayer::Defaults, "", &mut origins);

    for (layer, overlay) in [
        (ConfigLayer::Project, project),
        (ConfigLayer::Personal, personal),
    ] {
        if let Some(overlay) = overlay {
            merge_object(&mut values, overlay, layer, "", &mut origins);
        }
    }

    if !within_limits(&values) {
        return Err(vec![issue(
            extension,
            ConfigLayer::Resolved,
            "",
            "limit_exceeded",
        )]);
    }

    for (layer, value) in [
        (ConfigLayer::Defaults, defaults),
        (ConfigLayer::Resolved, &values),
    ] {
        for violation in validate(layer, value) {
            issues.push(ConfigIssue {
                extension: extension.to_owned(),
                layer,
                pointer: violation.pointer,
                code: violation.code,
            });
        }
    }

    if issues.is_empty() {
        if origins.is_empty() {
            origins.insert(String::new(), ConfigLayer::Defaults);
        }
        Ok(ResolvedConfig { values, origins })
    } else {
        Err(sort_issues(issues))
    }
}

fn check_layer(
    extension: &str,
    layer: ConfigLayer,
    value: Option<&serde_json::Value>,
    issues: &mut Vec<ConfigIssue>,
) {
    let Some(value) = value else {
        return;
    };
    if !value.is_object() {
        issues.push(issue(extension, layer, "", "object_required"));
        return;
    }
    if !within_limits(value) {
        issues.push(issue(extension, layer, "", "limit_exceeded"));
    }
}

fn within_limits(value: &serde_json::Value) -> bool {
    check_json_limits(value).is_ok()
}

/// Shared by merge resolution and the host schema adapter. Check depth before
/// serializing, and count bytes without allocating another copy of the input.
pub(super) fn check_json_limits(value: &serde_json::Value) -> Result<(), &'static str> {
    if exceeds_depth(value) {
        return Err("depth_limit");
    }
    serde_json::to_writer(&mut SizeCounter(0), value).map_err(|_| "size_limit")
}

struct SizeCounter(usize);

impl Write for SizeCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let size = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_CONFIG_BYTES)
            .ok_or_else(|| io::Error::other("configuration size limit"))?;
        self.0 = size;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn exceeds_depth(value: &serde_json::Value) -> bool {
    let mut pending = vec![(value, 1usize)];
    while let Some((value, current)) = pending.pop() {
        if current > MAX_CONFIG_DEPTH {
            return true;
        }
        match value {
            serde_json::Value::Object(map) => {
                pending.extend(map.values().map(|child| (child, current + 1)));
            }
            serde_json::Value::Array(items) => {
                pending.extend(items.iter().map(|child| (child, current + 1)));
            }
            _ => {}
        }
    }
    false
}

fn merge_object(
    base: &mut serde_json::Value,
    overlay: &serde_json::Value,
    layer: ConfigLayer,
    pointer: &str,
    origins: &mut BTreeMap<String, ConfigLayer>,
) {
    let (Some(base_map), Some(overlay_map)) = (base.as_object_mut(), overlay.as_object()) else {
        replace_subtree(base, overlay, layer, pointer, origins);
        return;
    };

    if !overlay_map.is_empty() {
        origins.remove(pointer);
    }
    for (key, value) in overlay_map {
        let child_pointer = format!("{pointer}/{}", escape_pointer(key));
        if let Some(existing) = base_map.get_mut(key) {
            merge_object(existing, value, layer, &child_pointer, origins);
        } else {
            base_map.insert(key.clone(), value.clone());
            assign_leaf_origins(value, layer, &child_pointer, origins);
        }
    }
}

fn replace_subtree(
    base: &mut serde_json::Value,
    replacement: &serde_json::Value,
    layer: ConfigLayer,
    pointer: &str,
    origins: &mut BTreeMap<String, ConfigLayer>,
) {
    if pointer.is_empty() {
        origins.clear();
    } else {
        origins.remove(pointer);
        let prefix = format!("{pointer}/");
        let descendants: Vec<_> = origins
            .range(prefix.clone()..)
            .take_while(|(path, _)| path.starts_with(&prefix))
            .map(|(path, _)| path.clone())
            .collect();
        for path in descendants {
            origins.remove(&path);
        }
    }
    *base = replacement.clone();
    assign_leaf_origins(replacement, layer, pointer, origins);
}

fn assign_leaf_origins(
    value: &serde_json::Value,
    layer: ConfigLayer,
    pointer: &str,
    origins: &mut BTreeMap<String, ConfigLayer>,
) {
    match value {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                assign_leaf_origins(
                    child,
                    layer,
                    &format!("{pointer}/{}", escape_pointer(key)),
                    origins,
                );
            }
        }
        _ => {
            origins.insert(pointer.to_owned(), layer);
        }
    }
}

fn escape_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn issue(extension: &str, layer: ConfigLayer, pointer: &str, code: &str) -> ConfigIssue {
    ConfigIssue {
        extension: extension.to_owned(),
        layer,
        pointer: pointer.to_owned(),
        code: code.to_owned(),
    }
}

fn layer_order(layer: ConfigLayer) -> u8 {
    match layer {
        ConfigLayer::Defaults => 0,
        ConfigLayer::Project => 1,
        ConfigLayer::Personal => 2,
        ConfigLayer::Resolved => 3,
    }
}

fn sort_issues(mut issues: Vec<ConfigIssue>) -> Vec<ConfigIssue> {
    issues.sort_by(|a, b| {
        (
            layer_order(a.layer),
            a.pointer.as_str(),
            a.code.as_str(),
            a.extension.as_str(),
        )
            .cmp(&(
                layer_order(b.layer),
                b.pointer.as_str(),
                b.code.as_str(),
                b.extension.as_str(),
            ))
    });
    issues.dedup();
    issues.truncate(MAX_CONFIG_ISSUES);
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolve(
        defaults: serde_json::Value,
        project: Option<serde_json::Value>,
        personal: Option<serde_json::Value>,
    ) -> Result<ResolvedConfig, Vec<ConfigIssue>> {
        resolve_extension_config(
            "alpha",
            &defaults,
            project.as_ref(),
            personal.as_ref(),
            |_, _| Vec::new(),
        )
    }

    #[test]
    fn precedence_is_defaults_then_project_then_personal() {
        let resolved = resolve(
            json!({"a": 1, "b": 1, "c": 1, "nested": {"x": 1, "y": 1}}),
            Some(json!({"b": 2, "c": 2, "nested": {"x": 2}})),
            Some(json!({"c": 3})),
        )
        .unwrap();
        assert_eq!(
            resolved.values,
            json!({"a": 1, "b": 2, "c": 3, "nested": {"x": 2, "y": 1}})
        );
        let origin = |pointer: &str| resolved.origins[pointer];
        assert_eq!(origin("/a"), ConfigLayer::Defaults);
        assert_eq!(origin("/b"), ConfigLayer::Project);
        assert_eq!(origin("/c"), ConfigLayer::Personal);
        assert_eq!(origin("/nested/x"), ConfigLayer::Project);
        assert_eq!(origin("/nested/y"), ConfigLayer::Defaults);
    }

    #[test]
    fn arrays_and_scalars_replace_and_missing_layers_inherit() {
        let resolved = resolve(
            json!({"list": [1, 2, 3], "name": "d", "obj": {"k": 1}}),
            None,
            Some(json!({"list": [9], "obj": {}})),
        )
        .unwrap();
        assert_eq!(
            resolved.values,
            json!({"list": [9], "name": "d", "obj": {"k": 1}}),
            "an empty overriding object merges without clearing"
        );
        assert_eq!(resolved.origins["/list"], ConfigLayer::Personal);
        assert_eq!(resolved.origins["/name"], ConfigLayer::Defaults);
    }

    #[test]
    fn a_layer_must_be_an_object_and_no_values_come_back_on_failure() {
        for layer in [json!(null), json!([1]), json!("s"), json!(5)] {
            let issues = resolve(json!({"a": 1}), Some(layer), None).unwrap_err();
            assert_eq!(issues[0].layer, ConfigLayer::Project);
            assert_eq!(issues[0].code, "object_required");
        }
    }

    #[test]
    fn validation_failures_carry_a_pointer_and_code_never_the_value() {
        let result = resolve_extension_config(
            "alpha",
            &json!({}),
            Some(&json!({"token": "hunter2"})),
            None,
            |layer, _| {
                if layer == ConfigLayer::Resolved {
                    vec![ConfigViolation {
                        pointer: "/token".into(),
                        code: "type".into(),
                    }]
                } else {
                    Vec::new()
                }
            },
        );
        let issues = result.unwrap_err();
        assert_eq!(
            issues,
            vec![ConfigIssue {
                extension: "alpha".into(),
                layer: ConfigLayer::Resolved,
                pointer: "/token".into(),
                code: "type".into(),
            }]
        );
        assert!(!format!("{issues:?}").contains("hunter2"));
    }

    #[test]
    fn an_invalid_namespace_is_refused_without_naming_it() {
        let issues =
            resolve_extension_config("Not Valid!", &json!({}), None, None, |_, _| Vec::new())
                .unwrap_err();
        assert_eq!(issues[0].code, "invalid_namespace");
        assert!(issues[0].extension.is_empty());
    }

    #[test]
    fn pointers_escape_slashes_and_tildes() {
        let resolved = resolve(json!({"a/b": 1, "c~d": 2}), None, None).unwrap();
        assert_eq!(resolved.origins["/a~1b"], ConfigLayer::Defaults);
        assert_eq!(resolved.origins["/c~0d"], ConfigLayer::Defaults);
    }
}
