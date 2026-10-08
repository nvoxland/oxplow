//! A throwaway oxplow (P7.C6): in memory, over a copy of a project's
//! `oxplow/extensions/`, with every declared entity published empty (as if
//! each collector had run and found nothing), the extension models
//! published and the commands registered as at boot. What `extension test`
//! runs everything on, and what `check` asks for a command registry when
//! no oxplow is running. It never touches the project's own data.

use std::path::{Path, PathBuf};

use oxplow_app::extensions::EXTENSIONS_DIR;

pub(crate) struct Host {
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) svc: oxplow_app::Services,
}

impl Host {
    pub(crate) async fn start(project: &Path) -> Result<Host, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().to_path_buf();
        let extensions = project.join(EXTENSIONS_DIR);
        if extensions.is_dir() {
            copy_dir(&extensions, &root.join(EXTENSIONS_DIR)).map_err(|e| e.to_string())?;
        }
        oxplow_app::vcs::GitProvider
            .init_repository(&root)
            .await
            .map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::in_memory(&root).map_err(|e| e.to_string())?;
        oxplow_app::collector_runner::publish_declared_empty(
            &svc.db,
            &svc.extension_catalog.get(&root),
        )
        .await
        .map_err(|e| e.to_string())?;
        svc.extension_models
            .sync()
            .await
            .map_err(|e| e.to_string())?;
        svc.extension_commands.reconcile().await;
        Ok(Host {
            _dir: dir,
            root,
            svc,
        })
    }
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
