//! Host-owned secrets. References are safe configuration data; neither a
//! reference nor imported configuration grants access to a keychain value.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxplow_ai::secrets::SecretStore;
use oxplow_domain::extension_secret::SecretRef;
use oxplow_domain::DomainError;

use crate::extension_catalog::ExtensionCatalog;

const MAX_SECRET_BYTES: usize = 16 * 1024;

/// Human settings ingress only. No command, script service or MCP adapter is
/// given this object. It returns references/presence, never secret values.
pub struct ExtensionSecrets {
    catalog: Arc<ExtensionCatalog>,
    root: PathBuf,
    project: String,
    secrets: Arc<dyn SecretStore>,
    sql: crate::sql_gateway::SqlGateway,
    revisions: parking_lot::Mutex<std::collections::BTreeMap<(String, String), Arc<AtomicU64>>>,
}

impl ExtensionSecrets {
    pub fn new(
        catalog: Arc<ExtensionCatalog>,
        root: &Path,
        secrets: Arc<dyn SecretStore>,
        sql: crate::sql_gateway::SqlGateway,
    ) -> Self {
        let root = catalog.project_of(root).to_path_buf();
        Self {
            project: crate::collector_runner::project_key(&root),
            catalog,
            root,
            secrets,
            sql,
            revisions: Default::default(),
        }
    }

    /// Create or rotate a secret under the selected current extension. The
    /// person may supply an inert imported reference when entering its value.
    pub async fn save(
        &self,
        extension: &str,
        reference: Option<SecretRef>,
        value: &str,
    ) -> Result<SecretRef, DomainError> {
        self.sql
            .read_generation(async { self.save_current(extension, reference, value) })
            .await?
    }

    fn save_current(
        &self,
        extension: &str,
        reference: Option<SecretRef>,
        value: &str,
    ) -> Result<SecretRef, DomainError> {
        self.current_owner(extension)?;
        if value.is_empty() || value.len() > MAX_SECRET_BYTES {
            return Err(DomainError::Invalid("invalid extension secret size".into()));
        }
        let reference = reference.unwrap_or_else(SecretRef::generate);
        let result = self
            .secrets
            .set(&account(&self.project, extension, &reference), value);
        self.changed(extension, &reference);
        result.map_err(|_| secret_failure())?;
        Ok(reference)
    }

    pub async fn is_set(
        &self,
        extension: &str,
        reference: &SecretRef,
    ) -> Result<bool, DomainError> {
        self.sql
            .read_generation(async {
                self.current_owner(extension)?;
                self.secrets
                    .get(&account(&self.project, extension, reference))
                    .map(|value| value.is_some())
                    .map_err(|_| secret_failure())
            })
            .await?
    }

    /// Explicit human revocation remains available after disable/uninstall.
    pub fn revoke(&self, extension: &str, reference: &SecretRef) -> Result<(), DomainError> {
        valid_owner(extension)?;
        let result = self
            .secrets
            .delete(&account(&self.project, extension, reference));
        self.changed(extension, reference);
        result.map_err(|_| secret_failure())
    }

    /// Process authority captures this revision at admission. Rotation and
    /// revocation invalidate it synchronously, even if settings RPC is dropped.
    pub(crate) fn watch(&self, extension: &str, reference: &SecretRef) -> Arc<AtomicU64> {
        self.revisions
            .lock()
            .entry((extension.to_owned(), reference.id().to_owned()))
            .or_default()
            .clone()
    }

    fn changed(&self, extension: &str, reference: &SecretRef) {
        if let Some(revision) = self
            .revisions
            .lock()
            .get(&(extension.to_owned(), reference.id().to_owned()))
        {
            revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn current_owner(&self, extension: &str) -> Result<(), DomainError> {
        valid_owner(extension)?;
        let snapshot = self.catalog.runtime_snapshot(&self.root);
        let current = snapshot
            .extensions()
            .iter()
            .find(|ext| ext.name == extension && ext.origin == "bundled")
            .or_else(|| {
                snapshot
                    .extensions()
                    .iter()
                    .find(|ext| ext.name == extension)
            });
        if !current.is_some_and(|ext| ext.enabled) {
            return Err(DomainError::Invalid(
                "extension secret owner is unavailable".into(),
            ));
        }
        Ok(())
    }
}

fn valid_owner(extension: &str) -> Result<(), DomainError> {
    if extension.len() <= 128 && oxplow_domain::contribution::valid_extension_name(extension) {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "invalid extension secret owner".into(),
        ))
    }
}

fn secret_failure() -> DomainError {
    // Keychain implementations may include account/platform details in errors.
    DomainError::Storage("extension secret storage is unavailable".into())
}

/// Trusted transport use only, after current owner and exact approval checks.
/// The caller supplies the primary project's identity, never a wire namespace.
pub(crate) fn account(project: &str, extension: &str, reference: &SecretRef) -> String {
    format!("extension-secret:{project}:{extension}:{}", reference.id())
}
