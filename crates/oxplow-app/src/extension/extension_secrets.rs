//! Host-owned secrets. References are safe configuration data; neither a
//! reference nor imported configuration grants access to a keychain value.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxplow_ai::secrets::SecretStore;
use oxplow_domain::extension_secret::SecretRef;
use oxplow_domain::DomainError;

use crate::extension::extension_catalog::ExtensionCatalog;

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
        let namespace = self.current_owner(extension)?;
        if value.is_empty() || value.len() > MAX_SECRET_BYTES {
            return Err(DomainError::Invalid("invalid extension secret size".into()));
        }
        let reference = reference.unwrap_or_else(SecretRef::generate);
        let result = self
            .secrets
            .set(&account(&self.project, &namespace, &reference), value);
        self.changed(&namespace, &reference);
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
                let namespace = self.current_owner(extension)?;
                self.secrets
                    .get(&account(&self.project, &namespace, reference))
                    .map(|value| value.is_some())
                    .map_err(|_| secret_failure())
            })
            .await?
    }

    /// Explicit human revocation remains available after disable/uninstall.
    pub fn revoke(&self, extension: &str, reference: &SecretRef) -> Result<(), DomainError> {
        valid_owner(extension)?;
        // One no longer here is revoked under the namespace its name makes.
        let namespace = crate::extension::extensions::owner(
            self.catalog.live_snapshot(&self.root).extensions(),
            extension,
        )
        .map_or_else(
            || oxplow_domain::contribution::extension_namespace(extension),
            |ext| ext.namespace.clone(),
        );
        let result = self
            .secrets
            .delete(&account(&self.project, &namespace, reference));
        self.changed(&namespace, reference);
        result.map_err(|_| secret_failure())
    }

    /// Process authority captures this revision at admission. Rotation and
    /// revocation invalidate it synchronously, even if settings RPC is dropped.
    /// Keyed by the namespace the secret is stored under.
    pub(crate) fn watch(&self, namespace: &str, reference: &SecretRef) -> Arc<AtomicU64> {
        self.revisions
            .lock()
            .entry((namespace.to_owned(), reference.id().to_owned()))
            .or_default()
            .clone()
    }

    fn changed(&self, namespace: &str, reference: &SecretRef) {
        if let Some(revision) = self
            .revisions
            .lock()
            .get(&(namespace.to_owned(), reference.id().to_owned()))
        {
            revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// `extension`, enabled here: the namespace its secrets are stored
    /// under.
    fn current_owner(&self, extension: &str) -> Result<String, DomainError> {
        valid_owner(extension)?;
        let snapshot = self.catalog.live_snapshot(&self.root);
        crate::extension::extensions::owner(snapshot.extensions(), extension)
            .filter(|ext| ext.enabled)
            .map(|ext| ext.namespace.clone())
            .ok_or_else(|| DomainError::Invalid("extension secret owner is unavailable".into()))
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
/// Stored under the extension's namespace, so a secret outlives a move
/// between siblings there.
pub(crate) fn account(project: &str, namespace: &str, reference: &SecretRef) -> String {
    format!("extension-secret:{project}:{namespace}:{}", reference.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::secrets::MemorySecrets;
    use oxplow_domain::extension_secret::SecretRef;

    /// A project with enabled extension `alpha`, over a memory keychain the
    /// test can inspect. The real keychain is never touched.
    async fn fixture() -> (Arc<crate::Services>, Arc<MemorySecrets>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let ext = dir.path().join(".oxplow/extensions/alpha");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: alpha\nintent:\n  purpose: p\n",
        )
        .unwrap();
        let keychain = Arc::new(MemorySecrets::default());
        let svc = crate::Services::in_memory_on_machine(
            dir.path(),
            dir.path().join(".oxplow/global-config"),
            keychain.clone(),
            crate::providers::host::process_env(),
        )
        .await
        .unwrap();
        crate::test_fixtures::activate(&svc).await;
        (svc, keychain, dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_saved_secret_lives_in_the_keychain_and_only_its_reference_comes_back() {
        let (svc, keychain, dir) = fixture().await;
        let secrets = &svc.extension_secrets;
        let reference = secrets.save("alpha", None, "s3cr3t-value").await.unwrap();
        assert!(secrets.is_set("alpha", &reference).await.unwrap());

        let project = crate::collector_runner::project_key(dir.path());
        assert_eq!(
            keychain
                .get(&account(&project, "alpha", &reference))
                .unwrap()
                .as_deref(),
            Some("s3cr3t-value")
        );
        assert!(!format!("{reference:?}").contains(reference.id()));

        // Rotating under the same reference replaces the value; another
        // extension's account for it is a different slot.
        let same = secrets
            .save("alpha", Some(reference.clone()), "rotated")
            .await
            .unwrap();
        assert_eq!(same, reference);
        assert_eq!(
            keychain
                .get(&account(&project, "alpha", &reference))
                .unwrap()
                .as_deref(),
            Some("rotated")
        );
        assert!(!secrets
            .is_set("alpha", &SecretRef::generate())
            .await
            .unwrap());

        secrets.revoke("alpha", &reference).unwrap();
        assert!(!secrets.is_set("alpha", &reference).await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rotation_and_revocation_bump_a_watched_revision() {
        let (svc, _keychain, _dir) = fixture().await;
        let secrets = &svc.extension_secrets;
        let reference = secrets.save("alpha", None, "one").await.unwrap();
        let watched = secrets.watch("alpha", &reference);
        let before = watched.load(Ordering::Acquire);
        secrets
            .save("alpha", Some(reference.clone()), "two")
            .await
            .unwrap();
        let rotated = watched.load(Ordering::Acquire);
        assert!(rotated > before);
        secrets.revoke("alpha", &reference).unwrap();
        assert!(watched.load(Ordering::Acquire) > rotated);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_secret_needs_a_current_owner_and_a_sane_value() {
        let (svc, keychain, _dir) = fixture().await;
        let secrets = &svc.extension_secrets;
        for owner in ["ghost", "Not Valid", ""] {
            assert!(
                secrets.save(owner, None, "v").await.is_err(),
                "{owner:?} owns nothing"
            );
        }
        assert!(secrets.save("alpha", None, "").await.is_err());
        assert!(secrets
            .save("alpha", None, &"x".repeat(MAX_SECRET_BYTES + 1))
            .await
            .is_err());
        assert!(
            keychain.get("anything").unwrap().is_none(),
            "nothing was stored for a refused save"
        );
    }
}
