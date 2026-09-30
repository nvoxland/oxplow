//! Extensions' SQL models (P4.9, `.context/semantic-layer.md` "Models"):
//! compiled from the primary worktree's enabled extensions into
//! `v_<extension>_<name>` views, after the core models, in one pass
//! (`oxplow_db::models::compile_extensions`). Views are project-wide, like
//! source data, so a worktree stream's extension edits reach them only
//! once merged.
//!
//! A pass runs at boot and again whenever what it compiles may have
//! changed: an edit under `oxplow/extensions/`, a config change (an
//! extension turned on or off), or a registry change (an extension's
//! source synced a new entity a model reads). The inputs are
//! fingerprinted, so a pass over the same models and entities — including
//! the one the pass's own registry writes set off — does nothing.
//!
//! A model that fails doesn't stop the others; its errors are its
//! extension's health, merged into the extension list, never a boot
//! failure.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::models::ExtensionModels;
use oxplow_db::Database;
use oxplow_domain::DomainError;
use tokio::sync::Mutex;

use crate::events::{EventBus, OxplowEvent};
use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::{Extension, EXTENSIONS_DIR};

/// A burst of file events is one pass.
const SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
struct State {
    fingerprint: Option<u64>,
    errors: BTreeMap<String, Vec<String>>,
}

pub struct ExtensionModelsService {
    db: Database,
    catalog: Arc<ExtensionCatalog>,
    /// The primary worktree, whose extensions publish models.
    root: PathBuf,
    state: Mutex<State>,
}

impl ExtensionModelsService {
    pub fn new(db: Database, catalog: Arc<ExtensionCatalog>, root: PathBuf) -> Self {
        Self {
            db,
            catalog,
            root,
            state: Mutex::new(State::default()),
        }
    }

    /// Compile the extensions' models if anything they depend on changed.
    /// Passes are serialized.
    pub async fn sync(&self) -> Result<(), DomainError> {
        let mut state = self.state.lock().await;
        let extensions: Vec<ExtensionModels> = self
            .catalog
            .get(&self.root)
            .iter()
            .filter(|e| e.enabled && !e.models.is_empty())
            .map(|e| ExtensionModels {
                extension: e.name.clone(),
                sources: e.models.clone(),
            })
            .collect();
        let entities: BTreeSet<String> = self
            .db
            .read(|tx| {
                let mut st = tx
                    .prepare("SELECT view FROM model WHERE kind = 'entity'")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<BTreeSet<_>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await?;
        let fingerprint = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(
                &extensions
                    .iter()
                    .map(|e| (&e.extension, &e.sources))
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| DomainError::Invalid(e.to_string()))?
            .hash(&mut h);
            entities.hash(&mut h);
            h.finish()
        };
        if state.fingerprint == Some(fingerprint) {
            return Ok(());
        }
        let errors = self.db.compile_extension_models(extensions).await?;
        for (extension, errs) in &errors {
            for e in errs {
                tracing::warn!(extension, error = %e, "extension model didn't compile");
            }
        }
        state.errors = errors;
        state.fingerprint = Some(fingerprint);
        Ok(())
    }

    /// `extensions` with each one's model errors added to its `errors` —
    /// for the primary worktree's list, where the models compile.
    pub async fn with_health(&self, root: &Path, mut extensions: Vec<Extension>) -> Vec<Extension> {
        if root != self.root {
            return extensions;
        }
        let state = self.state.lock().await;
        for ext in &mut extensions {
            if let Some(errs) = state.errors.get(&ext.name) {
                ext.errors.extend(errs.iter().cloned());
            }
        }
        extensions
    }

    /// Compile now, then again after each change that may matter, for the
    /// life of the process.
    pub fn spawn(self: Arc<Self>, events: EventBus) {
        let mut rx = events.subscribe();
        tokio::spawn(async move {
            if let Err(error) = self.sync().await {
                tracing::warn!(%error, "extension models didn't compile at boot");
            }
            loop {
                let relevant = match rx.recv().await {
                    Ok(OxplowEvent::WorkspaceChanged { path, .. }) => {
                        path.starts_with(&format!("{EXTENSIONS_DIR}/"))
                    }
                    Ok(OxplowEvent::ConfigChanged) => true,
                    Ok(OxplowEvent::ModelsChanged { models }) => {
                        models.iter().any(|m| m == "v_model")
                    }
                    Ok(_) => false,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                if !relevant {
                    continue;
                }
                tokio::time::sleep(SETTLE).await;
                while rx.try_recv().is_ok() {}
                if let Err(error) = self.sync().await {
                    tracing::warn!(%error, "extension models didn't compile");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const MANIFEST: &str = "manifest: 2\nname: late\nintent:\n  purpose: x\n  examples: [{ name: a }]\nmodels:\n  - name: blocked\n    version: 1\n    description: Blocked tasks.\n    columns:\n      - { name: id, type: INTEGER, doc: Task id. }\n  - name: broken\n    version: 1\n    description: Broken.\n    columns:\n      - { name: nope, doc: x }\n";

    /// P4.9 (tsk494): an extension's model publishes its view; one that
    /// doesn't compile is that extension's error in the list, not a boot
    /// failure; turning the extension off takes its views away.
    #[tokio::test]
    async fn extension_models_publish_and_report_their_errors() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write(&root, "oxplow/extensions/late/extension.yaml", MANIFEST);
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT id FROM ref('task') WHERE status = 'blocked'",
        );
        write(
            &root,
            "oxplow/extensions/late/models/broken.sql",
            "SELECT nope FROM ref('task')",
        );
        svc.extension_models.sync().await.unwrap();
        svc.sql
            .query_sql("SELECT id FROM v_late_blocked", vec![], None)
            .await
            .unwrap();
        let listed = svc
            .extension_models
            .with_health(&root, svc.extension_catalog.get(&root).to_vec())
            .await;
        let late = listed.iter().find(|e| e.name == "late").unwrap();
        assert!(
            late.errors.iter().any(|e| e.contains("models/broken.sql")),
            "{:?}",
            late.errors
        );
        // Off: its views go.
        write(
            &root,
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [late]\n",
        );
        svc.extension_models.sync().await.unwrap();
        assert!(svc
            .sql
            .query_sql("SELECT id FROM v_late_blocked", vec![], None)
            .await
            .is_err());
    }

    /// P4.9 (tsk494): `validate_extension` (what `oxplow plugin check`
    /// runs) catches a breaking change at a published version before it
    /// is published, and a plugin's `source()` of a core table.
    #[tokio::test]
    async fn validate_catches_a_breaking_change_and_a_foreign_source() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let manifest = |cols: &str| {
            format!(
                "manifest: 2\nname: late\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\nmodels:\n  - name: blocked\n    version: 1\n    description: Blocked tasks.\n    columns:\n{cols}"
            )
        };
        let id = "      - { name: id, type: INTEGER, doc: Task id. }\n";
        let title = "      - { name: title, type: TEXT, doc: Title. }\n";
        write(
            &root,
            "oxplow/extensions/late/extension.yaml",
            &manifest(id),
        );
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT id FROM ref('task') WHERE status = 'blocked'",
        );
        svc.extension_models.sync().await.unwrap();
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
            "SELECT id, title FROM ref('task') WHERE status = 'blocked'",
        );
        let errors = validate().await;
        assert!(
            errors.contains("column `title` added") && errors.contains("bump its version"),
            "{errors}"
        );
        write(
            &root,
            "oxplow/extensions/late/models/blocked.sql",
            "SELECT id, title FROM source('task')",
        );
        assert!(validate().await.contains("only its own extension's tables"));
    }
}
