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

use std::collections::{BTreeMap, HashMap};
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
    snapshot: Arc<CatalogSnapshot>,
}

/// Loaded declarations and the fingerprint of the load that owns them.
/// This pins catalog metadata, not database models or runtime registries.
pub struct CatalogSnapshot {
    extensions: Arc<Vec<Extension>>,
    generation: String,
    source_generation: String,
}

impl CatalogSnapshot {
    pub fn extensions(&self) -> &Arc<Vec<Extension>> {
        &self.extensions
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Fingerprint of the raw catalog source captured during preparation.
    /// Unlike `generation`, this is unchanged by synthetic runtime baselines
    /// and revocation overlays.
    pub fn source_generation(&self) -> &str {
        &self.source_generation
    }
    /// What a candidate activates: this load, with each enabled extension
    /// that can't load (its own errors, a blocked contract, or a required
    /// import of one that isn't there) replaced by its version in
    /// `published`, the generation active now, or left out when it has
    /// none. The rest activate together, so one broken extension never holds
    /// back the others. Required bundled extensions are never replaced:
    /// their errors still block. Returns the held extensions and why.
    ///
    /// `failed` names extensions whose preparation failed in an earlier
    /// attempt at this candidate: each is held the same way, and one whose
    /// last working version failed too stays out.
    pub(crate) fn admitted(
        &self,
        published: Option<&CatalogSnapshot>,
        failed: &Failed,
    ) -> (Arc<CatalogSnapshot>, BTreeMap<String, Vec<String>>) {
        let can_load = |e: &Extension| {
            e.errors.is_empty() && !e.public_contracts.blocked && !failed.0.contains_key(&e.name)
        };
        let may_keep_last = |e: &Extension| {
            e.errors.is_empty()
                && !e.public_contracts.blocked
                && failed.0.get(&e.name).is_none_or(|f| f.keep_last)
        };
        let mut held = BTreeMap::<String, Vec<String>>::new();
        let mut admitted: Vec<Extension> = Vec::with_capacity(self.extensions.len());
        for extension in self.extensions.iter() {
            // A contract-blocked extension is cleared to `enabled: false` by
            // the load, so it is told apart from a configured-off one here.
            if (!extension.enabled && !extension.public_contracts.blocked)
                || crate::bundled_extensions::is_required(&extension.name)
                || can_load(extension)
            {
                admitted.push(extension.clone());
                continue;
            }
            let last = published.and_then(|p| {
                p.extensions()
                    .iter()
                    .find(|e| e.name == extension.name && e.enabled && may_keep_last(e))
            });
            let mut why = extension.errors.clone();
            if let Some(f) = failed.0.get(&extension.name) {
                why.extend(f.reasons.iter().cloned());
            }
            match last {
                Some(last) => {
                    why.push("its last working version stays active until these are fixed".into());
                    admitted.push(last.clone());
                }
                None => why.push("it isn't active until these are fixed".into()),
            }
            held.insert(extension.name.clone(), why);
        }
        // An extension whose required import's owner is out is out too,
        // until nothing more drops.
        loop {
            let present: std::collections::BTreeSet<String> = admitted
                .iter()
                .filter(|e| e.enabled)
                .map(|e| e.name.clone())
                .collect();
            let missing = admitted.iter().position(|e| {
                e.enabled
                    && !crate::bundled_extensions::is_required(&e.name)
                    && e.public_contracts.imports.iter().any(|import| {
                        !import.optional && !present.contains(&import.contribution.extension)
                    })
            });
            let Some(index) = missing else { break };
            let out = admitted.remove(index);
            let owners: Vec<String> = out
                .public_contracts
                .imports
                .iter()
                .filter(|i| !i.optional && !present.contains(&i.contribution.extension))
                .map(|i| format!("`{}`", i.contribution.extension))
                .collect();
            held.entry(out.name.clone()).or_default().push(format!(
                "it imports from {}, which isn't active",
                owners.join(", ")
            ));
        }
        if held.is_empty() {
            return (
                Arc::new(CatalogSnapshot {
                    extensions: self.extensions.clone(),
                    generation: self.generation.clone(),
                    source_generation: self.source_generation.clone(),
                }),
                held,
            );
        }
        let names: Vec<&str> = held.keys().map(String::as_str).collect();
        let generation = format!(
            "{}+held({})@{}",
            self.source_generation,
            names.join(","),
            published.map_or("none", |p| p.generation())
        );
        (
            Arc::new(CatalogSnapshot {
                extensions: Arc::new(admitted),
                generation,
                source_generation: self.source_generation.clone(),
            }),
            held,
        )
    }

    /// A safe initial active generation while extension candidates are
    /// prepared. It contains only required bundled declarations; project and
    /// non-required bundled extensions remain available through raw snapshots.
    pub(crate) fn core_baseline(&self) -> Arc<CatalogSnapshot> {
        let extensions = self
            .extensions()
            .iter()
            .filter(|extension| {
                extension.origin == "bundled"
                    && crate::bundled_extensions::is_required(&extension.name)
            })
            .cloned()
            .collect();
        Arc::new(CatalogSnapshot {
            extensions: Arc::new(extensions),
            generation: format!("core:{}", self.source_generation()),
            source_generation: self.source_generation().to_string(),
        })
    }
}

/// Extensions whose preparation failed while one candidate was staged
/// (a model that doesn't compile, a provider that can't stage), held back
/// as one that can't load is (`CatalogSnapshot::admitted`).
#[derive(Default)]
pub(crate) struct Failed(BTreeMap<String, FailedExtension>);

struct FailedExtension {
    reasons: Vec<String>,
    /// Its last working version may stand in; false once that failed too.
    keep_last: bool,
}

impl Failed {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Hold the extensions `problems` names, if each is an enabled,
    /// non-required extension of `snapshot` that can still be held (a
    /// second failure takes its last working version out too). False when
    /// the problems aren't one extension's to hold: the candidate fails.
    pub(crate) fn hold(
        &mut self,
        snapshot: &CatalogSnapshot,
        problems: &BTreeMap<String, Vec<String>>,
    ) -> bool {
        let holdable = |name: &String| {
            !crate::bundled_extensions::is_required(name)
                && snapshot
                    .extensions()
                    .iter()
                    .any(|e| &e.name == name && e.enabled)
                && self.0.get(name).is_none_or(|f| f.keep_last)
        };
        if problems.is_empty() || !problems.keys().all(holdable) {
            return false;
        }
        for (name, reasons) in problems {
            self.0
                .entry(name.clone())
                .and_modify(|f| f.keep_last = false)
                .or_insert_with(|| FailedExtension {
                    reasons: reasons.clone(),
                    keep_last: true,
                });
        }
        true
    }
}

pub struct ExtensionCatalog {
    /// The project whose `.oxplow/project.yaml` governs every load (the
    /// daemon's main worktree). `None` standalone (the CLI, the SDK's
    /// checks): each root is its own project.
    project: Option<PathBuf>,
    by_root: Mutex<HashMap<PathBuf, Entry>>,
    /// Last atomically published primary-project declarations. Raw loads stay
    /// available for authoring and candidate validation.
    active: Mutex<Option<Arc<CatalogSnapshot>>>,
    /// Full loads performed; tests read it to prove a hit parses nothing.
    loads: AtomicUsize,
    /// The primary worktree's extensions may have changed.
    changes: tokio::sync::broadcast::Sender<()>,
    runtime_changes: tokio::sync::broadcast::Sender<()>,
}

impl Default for ExtensionCatalog {
    fn default() -> Self {
        Self {
            project: None,
            by_root: Mutex::default(),
            active: Mutex::default(),
            loads: AtomicUsize::default(),
            changes: tokio::sync::broadcast::channel(64).0,
            runtime_changes: tokio::sync::broadcast::channel(64).0,
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

    /// Replace the active primary-project catalog after coordinated
    /// generation publication. This never changes the raw load cache.
    /// The generation last activated, if one has been.
    pub(crate) fn active(&self) -> Option<Arc<CatalogSnapshot>> {
        self.active.lock().clone()
    }

    pub(crate) fn publish_active(&self, snapshot: Arc<CatalogSnapshot>) {
        *self.active.lock() = Some(snapshot);
        self.runtime_changed();
    }

    pub(crate) fn runtime_changed(&self) {
        let _ = self.runtime_changes.send(());
    }

    pub(crate) fn runtime_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.runtime_changes.subscribe()
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

    /// Runtime declarations under `root`. An activated primary project reads
    /// its published snapshot; authoring roots and standalone callers read
    /// current files. Use `snapshot` explicitly for candidate preparation.
    pub fn get(&self, root: &Path) -> Arc<Vec<Extension>> {
        self.runtime_snapshot(root).extensions.clone()
    }

    /// Take declarations and their load identity together. Retained
    /// snapshots keep their identity when a later load replaces the cache.
    pub fn snapshot(&self, root: &Path) -> Arc<CatalogSnapshot> {
        let project = self.project_of(root);
        let fingerprint = fingerprint(root, project);
        let mut cache = self.by_root.lock();
        if let Some(entry) = cache.get(root) {
            if entry.fingerprint == fingerprint {
                return entry.snapshot.clone();
            }
        }
        let extensions = Arc::new(extensions::load_extensions_in(root, project));
        let source_generation = fingerprint_generation(&fingerprint);
        let snapshot = Arc::new(CatalogSnapshot {
            extensions,
            generation: source_generation.clone(),
            source_generation,
        });
        self.loads.fetch_add(1, Ordering::Relaxed);
        cache.insert(
            root.to_path_buf(),
            Entry {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        snapshot
    }

    /// Runtime declarations for the primary project: the last coordinated
    /// snapshot, with only immediate disable/removal revocations applied.
    /// Other roots and the not-yet-activated state use the current raw load.
    /// `snapshot` remains the current-files API for authoring and staging.
    pub fn runtime_snapshot(&self, root: &Path) -> Arc<CatalogSnapshot> {
        let Some(project) = self.project.as_deref().filter(|project| *project == root) else {
            return self.snapshot(root);
        };
        let Some(active) = self.active.lock().clone() else {
            return self.snapshot(root);
        };

        let mut revoked = oxplow_config::disabled_extensions(project)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        revoked.retain(|name| !crate::bundled_extensions::is_required(name));
        for extension in active
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && extension.origin == "project")
        {
            let path = root.join(&extension.path);
            let still_present = std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                let kind = metadata.file_type();
                kind.is_dir() && !kind.is_symlink()
            });
            if !still_present {
                revoked.insert(extension.name.clone());
            }
        }

        if !active
            .extensions()
            .iter()
            .any(|extension| extension.enabled && revoked.contains(&extension.name))
        {
            return active;
        }

        let disabled: Vec<_> = revoked.into_iter().collect();
        Arc::new(CatalogSnapshot {
            extensions: Arc::new(extensions::resolve_catalog(
                active.extensions().as_ref().clone(),
                &disabled,
            )),
            generation: active.generation().to_string(),
            source_generation: active.source_generation().to_string(),
        })
    }

    pub(crate) fn candidate(
        &self,
        root: &Path,
        candidate: Extension,
    ) -> extensions::candidates::CandidateCatalog {
        extensions::candidates::build(root, self.project_of(root), candidate)
    }

    /// The generation of `root`'s catalog: a hash of everything its load
    /// depends on (every file under its extensions folder and the project
    /// config). This observes current files; a retained snapshot names its
    /// own load through `CatalogSnapshot::generation` instead.
    pub fn generation(&self, root: &Path) -> String {
        let fingerprint = fingerprint(root, self.project_of(root));
        fingerprint_generation(&fingerprint)
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
            if ext.public_contracts.blocked {
                return Err(DomainError::Invalid(format!(
                    "extension `{name}` cannot activate because its public contracts are invalid or unavailable: {}",
                    ext.errors.join("; ")
                )));
            }
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

fn fingerprint_generation(fingerprint: &Fingerprint) -> String {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    for (path, len, mtime) in fingerprint {
        h.update(path.to_string_lossy().as_bytes());
        h.update(&len.to_le_bytes());
        let since = mtime
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        h.update(&since.to_le_bytes());
    }
    format!("{:016x}", h.digest())
}

/// What a load of `root` depends on: every file under its extensions
/// folder, and `project`'s config (which can disable an extension).
fn fingerprint(root: &Path, project: &Path) -> Fingerprint {
    let mut out = Fingerprint::new();
    walk(&root.join(EXTENSIONS_DIR), &mut out, true);
    push_file(&oxplow_config::config_path(project), &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Fingerprint, extensions_root: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // The loader skips hidden extension folders. Publication staging
        // must not appear as a new loaded catalog generation either.
        if extensions_root && entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out, false),
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
    const LENS: &str = "title: Tasks\nquery: SELECT ref FROM v_work_item\nviz: table\n";

    fn names(snapshot: &CatalogSnapshot) -> Vec<String> {
        let mut names: Vec<String> = snapshot
            .extensions()
            .iter()
            .filter(|e| e.origin == "project")
            .map(|e| format!("{}:{}", e.name, e.description))
            .collect();
        names.sort();
        names
    }

    /// One extension that can't load doesn't hold back the rest: it keeps
    /// the version last activated, or stays out when it has none, and says
    /// why.
    #[test]
    fn a_broken_extension_is_left_out_of_what_activates() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = |name: &str, description: &str| {
            format!("manifest: 2\nname: {name}\ndescription: {description}\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\n")
        };
        write(
            dir.path(),
            "oxplow/extensions/good/extension.yaml",
            &manifest("good", "one"),
        );
        write(
            dir.path(),
            "oxplow/extensions/edited/extension.yaml",
            &manifest("edited", "one"),
        );
        let catalog = ExtensionCatalog::new();
        let (first, held) = catalog.snapshot(dir.path()).admitted(None, &Failed::new());
        assert!(held.is_empty(), "{held:?}");
        assert_eq!(names(&first), ["edited:one", "good:one"]);

        // `edited` breaks, `good` changes, `fresh` arrives broken.
        write(
            dir.path(),
            "oxplow/extensions/edited/extension.yaml",
            &format!("{}bogus: 1\n", manifest("edited", "two")),
        );
        write(
            dir.path(),
            "oxplow/extensions/good/extension.yaml",
            &manifest("good", "two"),
        );
        write(
            dir.path(),
            "oxplow/extensions/fresh/extension.yaml",
            &format!("{}bogus: 1\n", manifest("fresh", "one")),
        );
        let (next, held) = catalog
            .snapshot(dir.path())
            .admitted(Some(&first), &Failed::new());
        assert_eq!(names(&next), ["edited:one", "good:two"]);
        assert_eq!(held.keys().collect::<Vec<_>>(), ["edited", "fresh"]);
        assert!(
            held["edited"].iter().any(|e| e.contains("bogus")),
            "{held:?}"
        );
        assert!(
            held["edited"]
                .iter()
                .any(|e| e.contains("last working version stays active")),
            "{held:?}"
        );
        assert!(
            held["fresh"]
                .iter()
                .any(|e| e.contains("isn't active until")),
            "{held:?}"
        );
        assert_ne!(next.generation(), catalog.snapshot(dir.path()).generation());
    }

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
            "title: Tasks by status\nquery: SELECT ref FROM v_work_item\nviz: table\n",
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
