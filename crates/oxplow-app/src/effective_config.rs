//! The shared registry's redacted settings inventory, used by live SQL reads.

use std::{path::Path, sync::Arc};

use oxplow_config::settings::{SettingInventory, SettingsError};

use crate::extension::extensions::Extension;

/// Registry definitions and effective values. Secret values are never exposed;
/// their inventory rows retain only presence and origin metadata.
pub fn effective_config(
    project: &Path,
    global: &Path,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    extensions: &[Extension],
) -> Result<Vec<SettingInventory>, SettingsError> {
    crate::settings::store(project, global, secrets, extensions)?.inventory()
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
        crate::test_fixtures::write_settings_fixture(
            project.path().join(".oxplow/local.yaml"),
            "oxplow.snapshots.retentionDays: 14\n",
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
            .find(|row| row.definition.key == "oxplow.snapshots.retentionDays")
            .unwrap();
        assert_eq!(retention.origin, Some(Origin::Local));
        assert_eq!(retention.value, Some(json!(14)));
        assert_eq!(retention.definition.category, "snapshots");
        assert!(!retention.definition.title.is_empty());
        assert!(!retention.definition.schema.is_null());
        assert!(!rows
            .iter()
            .any(|row| row.definition.key == "metrics.gh.prs.open"));
    }
}
