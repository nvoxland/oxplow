//! The loaded-extensions cache (`.context/extensions.md` "Reading";
//! `.context/performance.md` "Extension catalog cache").
//!
//! `extensions::load_extensions(root)` parses every bundled and project
//! lens file on each call — ~3 ms — and it used to run on every
//! advisory check, extension listing and lens run. This caches the
//! result per worktree root behind a **stat-only fingerprint** of
//! `root/oxplow/extensions/**` and the project's `.oxplow/project.yaml`
//! (the main worktree's in the daemon, whatever the root; paths,
//! sizes, mtimes): a hit walks the tree with `stat` and parses nothing;
//! any edit, add or delete under the folder — or a change to the
//! project config that disables an extension — misses and reloads. No
//! watcher, no explicit invalidation, no window in which an agent's
//! fresh lens file is invisible. Consent hashing (`approval_hash`) keeps
//! reading the disk itself, so a cached `Extension` never stands in for
//! the bytes a person approved.
//!
//! **The change signal** (P7.B6) is for what follows the primary
//! worktree's extensions — their models, commands, providers and metric
//! declarations: [`ExtensionCatalog::changes`] hears that they may have
//! changed (a file under `oxplow/extensions/` in the primary worktree,
//! from the workspace watcher; the `extensions` config key, from its
//! `config.changed` reactor). The cache itself still needs no signal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oxplow_domain::DomainError;
use parking_lot::Mutex;

use crate::extensions::{self, Extension, Lens, EXTENSIONS_DIR};

/// `(path, len, mtime)` for every file the load depends on, sorted.
type Fingerprint = Vec<(PathBuf, u64, Option<std::time::SystemTime>)>;

struct Entry {
    fingerprint: Fingerprint,
    extensions: Arc<Vec<Extension>>,
}

pub struct ExtensionCatalog {
    /// The project whose `.oxplow/project.yaml` governs every load (the
    /// daemon's main worktree). `None` standalone (the CLI, the SDK's
    /// checks): each root is its own project.
    project: Option<PathBuf>,
    by_root: Mutex<HashMap<PathBuf, Entry>>,
    /// Full loads performed; tests read it to prove a hit parses nothing.
    loads: AtomicUsize,
    /// The primary worktree's extensions may have changed.
    changes: tokio::sync::broadcast::Sender<()>,
}

impl Default for ExtensionCatalog {
    fn default() -> Self {
        Self {
            project: None,
            by_root: Mutex::default(),
            loads: AtomicUsize::default(),
            changes: tokio::sync::broadcast::channel(64).0,
        }
    }
}

impl ExtensionCatalog {
    /// Standalone: each root it reads is its own project.
    pub fn new() -> Self {
        Self::default()
    }

    /// The daemon's: every root it reads — the main worktree, or a stream's
    /// working copy an authoring tool checks — takes its config
    /// (`extensions.disabled`) from `project_dir`, the main worktree.
    pub fn for_project(project_dir: &Path) -> Self {
        Self {
            project: Some(project_dir.to_path_buf()),
            ..Self::default()
        }
    }

    /// The project whose config governs a load of `root`.
    pub fn project_of<'a>(&'a self, root: &'a Path) -> &'a Path {
        self.project.as_deref().unwrap_or(root)
    }

    /// Hear that the primary worktree's extensions may have changed.
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// Say they may have: a file under the primary worktree's
    /// `oxplow/extensions/` changed, or the `extensions` config key.
    pub fn changed(&self) {
        let _ = self.changes.send(());
    }

    /// Every extension under `root` (bundled ones included), as
    /// `load_extensions` returns them, reloaded only when a file under
    /// `oxplow/extensions/` or the project config changed.
    pub fn get(&self, root: &Path) -> Arc<Vec<Extension>> {
        let project = self.project_of(root);
        let fingerprint = fingerprint(root, project);
        let mut cache = self.by_root.lock();
        if let Some(entry) = cache.get(root) {
            if entry.fingerprint == fingerprint {
                return entry.extensions.clone();
            }
        }
        let extensions = Arc::new(extensions::load_extensions_in(root, project));
        self.loads.fetch_add(1, Ordering::Relaxed);
        cache.insert(
            root.to_path_buf(),
            Entry {
                fingerprint,
                extensions: extensions.clone(),
            },
        );
        extensions
    }

    /// One extension by name. A disabled one is an error saying so; an
    /// unknown one is `NotFound`.
    pub fn named(&self, root: &Path, name: &str) -> Result<Extension, DomainError> {
        let all = self.get(root);
        // A project extension using a bundled name never shadows it.
        let ext = all
            .iter()
            .find(|e| e.name == name && e.origin == "bundled")
            .or_else(|| all.iter().find(|e| e.name == name))
            .ok_or(DomainError::NotFound)?;
        if !ext.enabled {
            return Err(extensions::disabled_error(name));
        }
        Ok(ext.clone())
    }

    /// One lens by `<extension>/<slug>`.
    pub fn find_lens(&self, root: &Path, id: &str) -> Result<Lens, DomainError> {
        let (ext, slug) = id.split_once('/').ok_or(DomainError::NotFound)?;
        self.named(root, ext)?
            .lenses
            .into_iter()
            .find(|l| l.slug == slug)
            .ok_or(DomainError::NotFound)
    }

    /// How many full loads have run. A hit doesn't count.
    pub fn loads(&self) -> usize {
        self.loads.load(Ordering::Relaxed)
    }
}

/// What a load of `root` depends on: every file under its extensions
/// folder, and `project`'s config (which can disable an extension).
fn fingerprint(root: &Path, project: &Path) -> Fingerprint {
    let mut out = Fingerprint::new();
    walk(&root.join(EXTENSIONS_DIR), &mut out);
    push_file(&oxplow_config::config_path(project), &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Fingerprint) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t) if t.is_file() || t.is_symlink() => push_file(&path, out),
            _ => {}
        }
    }
}

fn push_file(path: &Path, out: &mut Fingerprint) {
    if let Ok(meta) = std::fs::metadata(path) {
        out.push((path.to_path_buf(), meta.len(), meta.modified().ok()));
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

    const EXT: &str =
        "manifest: 2\nname: review\nintent:\n  purpose: x\n  examples: [{ name: a }]\n";
    const LENS: &str = "title: Tasks\nquery: SELECT id FROM v_task\nviz: table\n";

    #[test]
    fn a_hit_parses_nothing_and_an_edit_misses() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            LENS,
        );
        let catalog = ExtensionCatalog::new();
        let first = catalog.get(dir.path());
        assert_eq!(catalog.loads(), 1);
        let again = catalog.get(dir.path());
        assert!(
            Arc::ptr_eq(&first, &again),
            "the second call reuses the load"
        );
        assert_eq!(catalog.loads(), 1);
        assert_eq!(
            catalog.find_lens(dir.path(), "review/tasks").unwrap().title,
            "Tasks"
        );
        assert_eq!(catalog.loads(), 1);

        // A lens edit is seen on the very next call.
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/tasks.yaml",
            "title: Tasks by status\nquery: SELECT id FROM v_task\nviz: table\n",
        );
        assert_eq!(
            catalog.find_lens(dir.path(), "review/tasks").unwrap().title,
            "Tasks by status"
        );
        assert_eq!(catalog.loads(), 2);
        // So is a new lens file and a removed one.
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/more.yaml",
            LENS,
        );
        assert!(catalog.find_lens(dir.path(), "review/more").is_ok());
        std::fs::remove_file(dir.path().join("oxplow/extensions/review/lenses/more.yaml")).unwrap();
        assert!(matches!(
            catalog.find_lens(dir.path(), "review/more"),
            Err(DomainError::NotFound)
        ));
        assert_eq!(catalog.loads(), 4);
    }

    /// The daemon's catalog reads a stream's working copy for its files but
    /// the main worktree's `project.yaml` for what's enabled — and a change
    /// there is seen on the next read of the working copy.
    #[test]
    fn a_projects_catalog_takes_config_from_the_project_for_every_root() {
        let main = tempfile::tempdir().unwrap();
        let side = tempfile::tempdir().unwrap();
        write(side.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            side.path(),
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [oxplow-bundled]\n",
        );
        let catalog = ExtensionCatalog::for_project(main.path());
        assert!(catalog.named(side.path(), "review").is_ok());
        assert!(
            catalog.named(side.path(), "oxplow-bundled").is_ok(),
            "the working copy's own project.yaml doesn't count"
        );
        write(
            main.path(),
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [review]\n",
        );
        let err = catalog.named(side.path(), "review").unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
        assert_eq!(catalog.project_of(side.path()), main.path());
    }

    #[test]
    fn roots_are_cached_separately_and_project_config_counts() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            b.path(),
            "oxplow/extensions/board/extension.yaml",
            EXT.replace("review", "board").as_str(),
        );
        let catalog = ExtensionCatalog::new();
        assert!(catalog.named(a.path(), "review").is_ok());
        assert!(matches!(
            catalog.named(b.path(), "review"),
            Err(DomainError::NotFound)
        ));
        assert!(catalog.named(b.path(), "board").is_ok());
        assert_eq!(catalog.loads(), 2);
        // Disabling through project.yaml is a config change: a miss.
        write(
            a.path(),
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [review]\n",
        );
        let err = catalog.named(a.path(), "review").unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
        assert_eq!(catalog.loads(), 3);
        // A bundled extension resolves under every root without a project.
        assert_eq!(
            catalog.named(b.path(), "oxplow-bundled").unwrap().origin,
            "bundled"
        );
    }
}
