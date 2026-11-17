//! The shared registry's redacted settings inventory, used by live SQL reads.

use std::{path::Path, sync::Arc};

use oxplow_config::settings::{Level, SettingInventory, SettingsError};
use serde_json::Value;

use crate::extension::extensions::Extension;

/// Effective inventory and explicit ordinary overrides, in local/project/global
/// order. Secret values are absent from every override column.
pub struct SettingsRow {
    pub setting: SettingInventory,
    pub overrides: [Option<Value>; 3],
    pub statuses: [&'static str; 3],
}

/// Registry definitions and effective values. Secret values are never exposed;
/// their inventory rows retain only presence and origin metadata.
pub fn effective_config(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
) -> Result<Vec<SettingsRow>, SettingsError> {
    let documents = crate::settings::documents(project, global)?;
    let store =
        crate::settings::store_with_documents(project, global, secrets, extensions, &documents)?;
    let documents = Level::PRECEDENCE
        .into_iter()
        .zip(documents.iter().cloned())
        .collect::<Vec<_>>();
    Ok(store
        .inventory_snapshot_from(&documents)?
        .into_iter()
        .map(|snapshot| SettingsRow {
            setting: snapshot.setting,
            overrides: snapshot.overrides,
            statuses: snapshot.statuses,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::settings::Origin;
    use serde_json::json;

    #[test]
    fn inventory_uses_registry_metadata_and_effective_levels_without_metric_definitions() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".oxplow")).unwrap();
        std::fs::write(
            project.path().join(".oxplow/local.yaml"),
            "oxplow.snapshots.retentionDays: 14\n",
        )
        .unwrap();
        std::fs::write(
            project.path().join(".oxplow/project.yaml"),
            "oxplow.snapshots.retentionDays: 20\n",
        )
        .unwrap();
        std::fs::write(
            global.path().join("global.yaml"),
            "oxplow.snapshots.retentionDays: 30\n",
        )
        .unwrap();
        let mut extension = crate::extension::extensions::empty_extension("gh", "gh", "project");
        extension.metrics = vec![serde_json::from_value(json!({"key": "gh.prs.open"})).unwrap()];
        let rows = effective_config(
            project.path(),
            global.path(),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            &[extension],
        )
        .unwrap();
        let retention = rows
            .iter()
            .find(|row| row.setting.definition.key == "oxplow.snapshots.retentionDays")
            .unwrap();
        assert_eq!(retention.setting.origin, Some(Origin::Local));
        assert_eq!(retention.setting.value, Some(json!(14)));
        assert_eq!(
            retention.overrides,
            [Some(json!(14)), Some(json!(20)), Some(json!(30))]
        );
        assert_eq!(retention.setting.definition.category, "snapshots");
        assert!(!retention.setting.definition.title.is_empty());
        assert!(!retention.setting.definition.schema.is_null());
        assert!(!rows
            .iter()
            .any(|row| row.setting.definition.key == "metrics.gh.prs.open"));
    }
    #[test]
    fn explicit_level_columns_never_contain_secure_values() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let secrets = Arc::new(oxplow_ai::secrets::MemorySecrets::default());
        let extensions = crate::test_fixtures::shipped_extensions();
        let store =
            crate::settings::store(project.path(), global.path(), secrets.clone(), &extensions)
                .unwrap();
        store
            .set(
                "oxplow.ai.openai.baseUrl",
                json!("https://inventory.test/v1"),
                Level::Project,
            )
            .unwrap();
        let key = crate::ai_service::provider_key_setting("openai");
        store
            .set(&key, json!("inventory-hidden-marker"), Level::Local)
            .unwrap();
        let rows = effective_config(project.path(), global.path(), secrets, &extensions).unwrap();
        let row = rows
            .iter()
            .find(|row| row.setting.definition.key == key)
            .unwrap();
        assert!(row.setting.is_set && !row.setting.unavailable);
        assert!(row.setting.value.is_none());
        assert_eq!(row.overrides, [None, None, None]);
    }
}
