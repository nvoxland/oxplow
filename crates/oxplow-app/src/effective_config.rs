//! Settings as a view (P6.H1, target §13.3.4): every setting that shapes
//! this project, with its value and where it comes from — the project's
//! `.oxplow/project.yaml`, the person's global config (`ai.yaml`, the
//! global metric manifests), an enabled extension, or the default. The
//! Settings page lists these; a person asks the agent to change one, or
//! changes a person-only one themselves. See `.context/commands.md` →
//! "`config.*` and the key registry".

use std::path::Path;

use oxplow_config::OxplowConfig;
use serde_json::Value;

use crate::ai_service::AiSettings;
use crate::extensions::Extension;

/// Where a setting's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ConfigOrigin {
    Default,
    Global,
    Project,
    Extension,
}

/// One setting as the Settings view shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveSetting {
    /// A project key (`snapshotRetentionDays`), or a scoped name
    /// (`ai.roles.main`, `metrics.<key>`, `dimensions.<key>`).
    pub key: String,
    pub doc: String,
    #[specta(type = oxplow_domain::Json)]
    pub value: Value,
    pub origin: ConfigOrigin,
    /// The extension a value comes from (`origin: extension`).
    pub extension: Option<String>,
    /// Only a person may set it; an agent's change asks them.
    pub human_only: bool,
    /// A project key's value schema; null for the rest.
    #[specta(type = oxplow_domain::Json)]
    pub schema: Value,
}

fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    serde_json::from_str(&serde_json::to_string(v).expect("yaml serializes as json"))
        .expect("json parses")
}

fn entry_key<T: serde::Serialize>(entry: &T) -> Option<String> {
    serde_json::to_value(entry)
        .ok()?
        .get("key")?
        .as_str()
        .map(str::to_string)
}

fn plain<T: serde::Serialize>(entry: &T) -> Value {
    serde_json::from_str(&serde_json::to_string(entry).expect("entry serializes"))
        .expect("json parses")
}

/// Every setting: the project keys (value, and `project` when the file
/// sets it, else `default` with the default's value), the AI roles
/// (`project` when the project overrides one, `global` from `ai.yaml`,
/// `default` when unbound), metrics and dimensions from the global
/// manifests and from enabled extensions.
pub fn effective_config(
    config: &OxplowConfig,
    project_dir: &Path,
    ai: Option<&AiSettings>,
    global_dir: Option<&Path>,
    extensions: &[Extension],
) -> Vec<EffectiveSetting> {
    let fallback = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let keys = oxplow_config::keys::config_keys();
    let mut out: Vec<EffectiveSetting> = oxplow_config::config_entries(config, &fallback)
        .into_iter()
        .map(|e| {
            let key = keys.iter().find(|k| k.key == e.key);
            EffectiveSetting {
                key: e.key.to_string(),
                doc: key.map(|k| k.doc.clone()).unwrap_or_default(),
                value: yaml_to_json(&e.value),
                origin: if e.set {
                    ConfigOrigin::Project
                } else {
                    ConfigOrigin::Default
                },
                extension: None,
                human_only: key.is_some_and(|k| k.human_only),
                schema: key.map(|k| k.schema.clone()).unwrap_or(Value::Null),
            }
        })
        .collect();
    if let Some(ai) = ai {
        for r in &ai.roles {
            let role = serde_json::to_value(r.role)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            out.push(EffectiveSetting {
                key: format!("ai.roles.{role}"),
                doc: format!("The model the `{role}` AI role uses (a provider and a model)."),
                value: r.binding.as_ref().map(plain).unwrap_or(Value::Null),
                origin: match (&r.binding, r.overridden) {
                    (_, true) => ConfigOrigin::Project,
                    (Some(_), false) => ConfigOrigin::Global,
                    (None, false) => ConfigOrigin::Default,
                },
                extension: None,
                human_only: true,
                schema: Value::Null,
            });
        }
    }
    let mut catalog = |kind: &str,
                       doc: &str,
                       key: Option<String>,
                       value: Value,
                       origin,
                       extension: Option<String>| {
        if let Some(key) = key {
            out.push(EffectiveSetting {
                key: format!("{kind}.{key}"),
                doc: doc.to_string(),
                value,
                origin,
                extension,
                human_only: false,
                schema: Value::Null,
            });
        }
    };
    if let Some(dir) = global_dir {
        for m in oxplow_config::load_global_metric_entries(dir) {
            catalog(
                "metrics",
                "A metric defined in your global metric manifests.",
                entry_key(&m),
                plain(&m),
                ConfigOrigin::Global,
                None,
            );
        }
        for d in oxplow_config::load_global_dimension_entries(dir) {
            catalog(
                "dimensions",
                "A dimension defined in your global manifests.",
                entry_key(&d),
                plain(&d),
                ConfigOrigin::Global,
                None,
            );
        }
    }
    for ext in extensions.iter().filter(|e| e.enabled) {
        for m in &ext.metrics {
            catalog(
                "metrics",
                "A metric this extension contributes.",
                entry_key(m),
                plain(m),
                ConfigOrigin::Extension,
                Some(ext.name.clone()),
            );
        }
        for d in &ext.dimensions {
            catalog(
                "dimensions",
                "A dimension this extension contributes.",
                entry_key(d),
                plain(d),
                ConfigOrigin::Extension,
                Some(ext.name.clone()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_service::RoleStatus;
    use oxplow_ai::config::{Role, RoleBinding};

    fn find<'a>(all: &'a [EffectiveSetting], key: &str) -> &'a EffectiveSetting {
        all.iter()
            .find(|s| s.key == key)
            .unwrap_or_else(|| panic!("no {key}"))
    }

    /// P6.H1: a project-set key is `project`; an unset one is `default`,
    /// with the default's value; a global AI role is `global` and one the
    /// project overrides `project`; an extension's metric is `extension`.
    #[test]
    fn every_setting_says_where_its_value_comes_from() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/project.yaml"),
            "snapshotRetentionDays: 14\n",
        )
        .unwrap();
        let config = oxplow_config::load_project_config(dir.path()).unwrap();
        let ai = AiSettings {
            providers: vec![],
            roles: vec![
                RoleStatus {
                    role: Role::Main,
                    binding: Some(RoleBinding {
                        provider: "p".into(),
                        model: "m".into(),
                    }),
                    overridden: false,
                },
                RoleStatus {
                    role: Role::Fast,
                    binding: Some(RoleBinding {
                        provider: "p".into(),
                        model: "f".into(),
                    }),
                    overridden: true,
                },
                RoleStatus {
                    role: Role::Embed,
                    binding: None,
                    overridden: false,
                },
            ],
        };
        let mut ext = crate::extensions::empty_extension("gh", "gh", "project");
        ext.metrics =
            vec![serde_json::from_value(serde_json::json!({ "key": "gh.prs.open" })).unwrap()];
        let all = effective_config(&config, dir.path(), Some(&ai), None, &[ext]);

        let set = find(&all, "snapshotRetentionDays");
        assert_eq!(
            (set.origin, set.value.clone()),
            (ConfigOrigin::Project, serde_json::json!(14))
        );
        let unset = find(&all, "metricDetailRetentionDays");
        assert_eq!(
            (unset.origin, unset.value.clone()),
            (ConfigOrigin::Default, serde_json::json!(30))
        );
        assert!(!unset.doc.is_empty() && !unset.schema.is_null());
        assert_eq!(find(&all, "ai.roles.main").origin, ConfigOrigin::Global);
        assert_eq!(find(&all, "ai.roles.fast").origin, ConfigOrigin::Project);
        assert_eq!(find(&all, "ai.roles.embed").origin, ConfigOrigin::Default);
        assert!(find(&all, "ai.roles.main").human_only);
        let metric = find(&all, "metrics.gh.prs.open");
        assert_eq!(
            (metric.origin, metric.extension.as_deref()),
            (ConfigOrigin::Extension, Some("gh"))
        );
    }
}
