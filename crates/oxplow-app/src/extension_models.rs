//! Extensions' SQL models (P4.9, `.context/semantic-layer.md` "Models"):
//! compiled from the primary worktree's enabled extensions into
//! `v_<extension>_<name>` views, after the core models, in one pass
//! (`oxplow_db::models::compile_extensions`). Views are project-wide, like
//! source data, so a worktree stream's extension edits reach them only
//! once merged.
//!
//! A collector run (`collector.synced`, the `extension_models.entities`
//! consumer) may have registered an entity a model reads: it asks the
//! generation owner for a new generation when the entity signature moved.
//! Compiling is the generation's (`extension_activation.rs`); a model that
//! fails holds back its extension, never boot.

use std::sync::Arc;

use oxplow_domain::DomainError;

/// `extension_models.entities` (P7.B6): a collector run may have
/// registered an entity a model reads, so the models compile again (a
/// no-op when nothing they read changed).
pub const ENTITIES: &str = "extension_models.entities";

pub(crate) type EntitySignature = Vec<(String, i64, String, String, String)>;

pub(crate) fn entity_signature_tx(
    tx: &rusqlite::Transaction<'_>,
) -> Result<EntitySignature, DomainError> {
    let mut statement = tx
        .prepare(
            "SELECT m.view, m.version, m.sql, COALESCE(c.columns_json, ''),
                COALESCE(c.key_json, '')
         FROM model m LEFT JOIN model_contract c
           ON c.view = m.view AND c.version = m.version
         WHERE m.kind = 'entity' ORDER BY m.view",
        )
        .map_err(oxplow_db::map_sql_err)?;
    let signature = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(oxplow_db::map_sql_err)?
        .collect::<rusqlite::Result<EntitySignature>>()
        .map_err(oxplow_db::map_sql_err)?;
    Ok(signature)
}

pub struct EntitiesConsumer {
    services: std::sync::Weak<crate::Services>,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for EntitiesConsumer {
    fn name(&self) -> &'static str {
        ENTITIES
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "collector.synced"
    }

    async fn handle(&self, _event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        if let Some(svc) = self.services.upgrade() {
            svc.extension_activation
                .request_entity_refresh(&svc.db)
                .await?;
        }
        Ok(())
    }
}

pub fn register(state: &Arc<crate::Services>) {
    state.event_pump.register_async(Arc::new(EntitiesConsumer {
        services: Arc::downgrade(state),
    }));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const MANIFEST: &str = "manifest: 2\nname: late\nintent:\n  purpose: x\n  examples: [{ name: a }]\nmodels:\n  - name: blocked\n    version: 1\n    description: Blocked work items.\n    columns:\n      - { name: ref, type: TEXT, doc: Work item ref. }\n  - name: broken\n    version: 1\n    description: Broken.\n    columns:\n      - { name: nope, doc: x }\n";

    /// P4.9 (tsk494): an extension's model publishes its view; one that
    /// doesn't compile is that extension's error in the list, not a boot
    /// failure; turning the extension off takes its views away.
    #[tokio::test]
    async fn extension_models_publish_and_report_their_errors() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        // `late` has a good model, `shaky` one that doesn't compile: the
        // broken extension is held back with its error, the other publishes.
        let (good, broken) = MANIFEST.split_once("  - name: broken").unwrap();
        write(&root, "oxplow/extensions/late/extension.yaml", good);
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT ref FROM ref('work_item') WHERE state = 'blocked'",
        );
        write(
            &root,
            "oxplow/extensions/shaky/extension.yaml",
            &format!(
                "{}  - name: broken{broken}",
                good.replace("name: late", "name: shaky")
                    .split("  - name: blocked")
                    .next()
                    .unwrap()
            ),
        );
        write(
            &root,
            "oxplow/extensions/shaky/models/broken.sql",
            "SELECT nope FROM ref('work_item')",
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        svc.sql
            .query_sql("SELECT ref FROM v_late_blocked", vec![], None)
            .await
            .unwrap();
        let listed = svc.listed_extensions(&root).await;
        let shaky = listed.iter().find(|e| e.name == "shaky").unwrap();
        assert!(
            shaky.errors.iter().any(|e| e.contains("models/broken.sql")),
            "{:?}",
            shaky.errors
        );
        // Off: its views go.
        write(
            &root,
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [late]\n",
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        assert!(svc
            .sql
            .query_sql("SELECT ref FROM v_late_blocked", vec![], None)
            .await
            .is_err());
    }

    /// P4.9 (tsk494): `validate_extension` (what `oxplow extension check`
    /// runs) catches a breaking change at a published version before it
    /// is published, and an extension's `source()` of a core table.
    #[tokio::test]
    async fn validate_catches_a_breaking_change_and_a_foreign_source() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let manifest = |cols: &str| {
            format!(
                "manifest: 2\nname: late\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\nmodels:\n  - name: blocked\n    version: 1\n    description: Blocked work items.\n    columns:\n{cols}"
            )
        };
        let id = "      - { name: ref, type: TEXT, doc: Work item ref. }\n";
        let title = "      - { name: title, type: TEXT, doc: Title. }\n";
        write(
            &root,
            "oxplow/extensions/late/extension.yaml",
            &manifest(id),
        );
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT ref FROM ref('work_item') WHERE state = 'blocked'",
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        let validate = || async {
            crate::extensions::validate_extension(
                &svc.sql,
                &svc.extension_catalog,
                &root,
                "late",
                None,
            )
            .await
            .unwrap()
            .errors
            .join("\n")
        };
        assert_eq!(validate().await, "");
        write(
            &root,
            "oxplow/extensions/late/extension.yaml",
            &manifest(&format!("{id}{title}")),
        );
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT ref, title FROM ref('work_item') WHERE state = 'blocked'",
        );
        let errors = validate().await;
        assert!(
            errors.contains("column `title` added") && errors.contains("bump its version"),
            "{errors}"
        );
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT ref, title FROM source('work_item')",
        );
        assert!(validate().await.contains("only its own extension's tables"));
    }
}
