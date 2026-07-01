//! Mason-registry-backed LSP installer wired into Services.
//!
//! Wraps `oxplow_lsp_installer::{Registry, Installer}` and keeps a
//! small JSON manifest at `<state>/lsp/installed.json` so installed
//! servers re-register with `LspSessionManager` on boot — otherwise
//! the renderer would have to call `install_lsp_package` again every
//! time the runtime restarts.
//!
//! Design notes:
//!   - One install dir per project (`.oxplow/lsp/`). Worktrees stay
//!     isolated per the architecture rule. We pay the install cost
//!     per stream — fine for the prototype.
//!   - Mason language strings (e.g. "Rust") are folded to lowercase
//!     and trimmed before being stored as LSP `languageId`. Mason
//!     uses Title Case; oxplow stores ids lowercase.
//!   - The manifest is best-effort: if it gets corrupted, we drop it
//!     and re-discover packages on next install. No migrations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

use oxplow_config::LspServerConfig;
use oxplow_lsp_installer::{
    current_target, InstallError, Installed, Installer as MasonInstaller, Registry, RegistryError,
    Target,
};

use crate::lsp_sessions::LspSessionManager;

#[derive(Debug, Error)]
pub enum LspInstallerError {
    #[error("registry: {0}")]
    Registry(#[from] RegistryError),
    #[error("install: {0}")]
    Install(#[from] InstallError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("package `{0}` declares no languages — cannot register an lsp client")]
    NoLanguages(String),
}

/// Persisted entry in `installed.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledManifestEntry {
    pub name: String,
    pub version: String,
    pub language_ids: Vec<String>,
    pub binary: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    entries: HashMap<String, InstalledManifestEntry>,
}

#[derive(Clone)]
pub struct LspInstallerService {
    install_root: PathBuf,
    cache_root: PathBuf,
    manifest_path: PathBuf,
    inner: Arc<Mutex<()>>,
    sessions: LspSessionManager,
}

impl LspInstallerService {
    /// Construct from a state directory (typically `.oxplow/`).
    /// Doesn't touch the disk yet — call `replay_into_sessions` after
    /// boot to push existing entries into the session manager.
    pub fn new(state_dir: &Path, sessions: LspSessionManager) -> Self {
        let install_root = state_dir.join("lsp");
        let cache_root = state_dir.join("lsp-cache");
        let manifest_path = install_root.join("installed.json");
        Self {
            install_root,
            cache_root,
            manifest_path,
            inner: Arc::new(Mutex::new(())),
            sessions,
        }
    }

    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    /// Read the on-disk manifest and register every entry with the
    /// session manager. Idempotent; called at boot.
    pub async fn replay_into_sessions(&self) -> Result<(), LspInstallerError> {
        let manifest = self.read_manifest().await?;
        for entry in manifest.entries.values() {
            for lang in &entry.language_ids {
                self.sessions.installed_servers().register(
                    LspServerConfig {
                        language_id: lang.clone(),
                        extensions: vec![],
                        command: entry.binary.to_string_lossy().to_string(),
                        args: vec![],
                    },
                    &entry.name,
                    &entry.version,
                );
            }
        }
        info!(
            count = manifest.entries.len(),
            "lsp installer manifest replayed"
        );
        Ok(())
    }

    /// Install (or reinstall) a Mason package by name. Downloads from
    /// the registry, extracts under `.oxplow/lsp/<name>/`, registers
    /// with the session manager, and persists the manifest.
    pub async fn install(
        &self,
        package_name: &str,
    ) -> Result<InstalledManifestEntry, LspInstallerError> {
        let _guard = self.inner.lock().await;
        let registry = Registry::new(self.cache_root.clone());
        let installer = MasonInstaller::new(self.install_root.clone());
        let pkg = registry.fetch_package(package_name).await?;
        let target = current_target();
        let installed = installer.install(&pkg, &target).await?;
        let entry = self.persist(installed, &target).await?;
        for lang in &entry.language_ids {
            self.sessions.installed_servers().register(
                LspServerConfig {
                    language_id: lang.clone(),
                    extensions: vec![],
                    command: entry.binary.to_string_lossy().to_string(),
                    args: vec![],
                },
                &entry.name,
                &entry.version,
            );
        }
        info!(package = package_name, languages = ?entry.language_ids, "lsp package installed");
        Ok(entry)
    }

    /// Remove an installed package: delete its install directory, drop
    /// it from the manifest, and unregister its language servers. A
    /// package that isn't installed is a no-op (idempotent).
    pub async fn remove(&self, package_name: &str) -> Result<(), LspInstallerError> {
        let _guard = self.inner.lock().await;
        let mut manifest = self.read_manifest().await?;
        manifest.entries.remove(package_name);
        self.write_manifest(&manifest).await?;
        let dir = self.install_root.join(package_name);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.sessions
            .installed_servers()
            .unregister_package(package_name);
        info!(package = package_name, "lsp package removed");
        Ok(())
    }

    pub async fn list_installed(&self) -> Result<Vec<InstalledManifestEntry>, LspInstallerError> {
        let manifest = self.read_manifest().await?;
        let mut out: Vec<_> = manifest.entries.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn persist(
        &self,
        installed: Installed,
        _target: &Target,
    ) -> Result<InstalledManifestEntry, LspInstallerError> {
        if installed.languages.is_empty() {
            return Err(LspInstallerError::NoLanguages(installed.name.clone()));
        }
        let language_ids: Vec<String> = installed
            .languages
            .iter()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect();
        let entry = InstalledManifestEntry {
            name: installed.name,
            version: installed.version,
            language_ids,
            binary: installed.binary,
        };
        let mut manifest = self.read_manifest().await.unwrap_or_default();
        manifest.entries.insert(entry.name.clone(), entry.clone());
        self.write_manifest(&manifest).await?;
        Ok(entry)
    }

    async fn read_manifest(&self) -> Result<Manifest, LspInstallerError> {
        match tokio::fs::read(&self.manifest_path).await {
            Ok(bytes) => match serde_json::from_slice::<Manifest>(&bytes) {
                Ok(m) => Ok(m),
                Err(e) => {
                    warn!(?e, "lsp installer manifest unreadable; starting fresh");
                    Ok(Manifest::default())
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(e.into()),
        }
    }

    async fn write_manifest(&self, manifest: &Manifest) -> Result<(), LspInstallerError> {
        if let Some(parent) = self.manifest_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_vec_pretty(manifest)?;
        tokio::fs::write(&self.manifest_path, json).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;
    use tempfile::tempdir;

    fn empty_session_mgr() -> LspSessionManager {
        let cfg = Arc::new(RwLock::new(oxplow_config::OxplowConfig {
            agents: vec![oxplow_config::AgentKind::Claude],
            project_name: "p".into(),
            lsp_servers: vec![],
            agent_prompt_append: String::new(),
            snapshot_retention_days: 7,
            generated: oxplow_config::GeneratedConfig::default(),
            snapshot_max_file_bytes: 0,
            inject_session_context: true,
            collection: Default::default(),
            metrics: Default::default(),
            gauges: Default::default(),
            measures: Default::default(),
            dimensions: Default::default(),
            agent_models: Default::default(),
        }));
        LspSessionManager::new(cfg)
    }

    #[tokio::test]
    async fn replay_with_no_manifest_is_noop() {
        let tmp = tempdir().unwrap();
        let svc = LspInstallerService::new(tmp.path(), empty_session_mgr());
        svc.replay_into_sessions().await.unwrap();
        assert!(svc.list_installed().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn replay_registers_persisted_entries() {
        let tmp = tempdir().unwrap();
        let mgr = empty_session_mgr();
        let svc = LspInstallerService::new(tmp.path(), mgr.clone());
        // Seed manifest directly.
        let entry = InstalledManifestEntry {
            name: "rust-analyzer".into(),
            version: "v1".into(),
            language_ids: vec!["rust".into()],
            binary: tmp.path().join("lsp/rust-analyzer/rust-analyzer"),
        };
        let manifest = Manifest {
            entries: HashMap::from([("rust-analyzer".to_string(), entry)]),
        };
        svc.write_manifest(&manifest).await.unwrap();
        svc.replay_into_sessions().await.unwrap();
        let listed = mgr.installed_servers().list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].config.language_id, "rust");
        assert_eq!(listed[0].package, "rust-analyzer");
        assert_eq!(listed[0].version, "v1");
    }

    #[tokio::test]
    async fn remove_drops_manifest_entry_dir_and_registration() {
        let tmp = tempdir().unwrap();
        let mgr = empty_session_mgr();
        let svc = LspInstallerService::new(tmp.path(), mgr.clone());
        let install_dir = tmp.path().join("lsp/rust-analyzer");
        tokio::fs::create_dir_all(&install_dir).await.unwrap();
        let binary = install_dir.join("rust-analyzer");
        tokio::fs::write(&binary, b"#!/bin/sh\n").await.unwrap();
        let entry = InstalledManifestEntry {
            name: "rust-analyzer".into(),
            version: "v1".into(),
            language_ids: vec!["rust".into()],
            binary,
        };
        let manifest = Manifest {
            entries: HashMap::from([("rust-analyzer".to_string(), entry)]),
        };
        svc.write_manifest(&manifest).await.unwrap();
        svc.replay_into_sessions().await.unwrap();
        assert_eq!(mgr.installed_servers().list().len(), 1);

        svc.remove("rust-analyzer").await.unwrap();
        assert!(svc.list_installed().await.unwrap().is_empty());
        assert!(mgr.installed_servers().list().is_empty());
        assert!(!install_dir.exists());
        // Idempotent on a package that isn't installed.
        svc.remove("rust-analyzer").await.unwrap();
    }
}
