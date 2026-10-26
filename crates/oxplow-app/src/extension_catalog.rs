//! The loaded extensions (`.context/extensions.md` "Deployments";
//! `.context/performance.md` "Extension catalog cache").
//!
//! **Deployed** (the daemon's, [`ExtensionCatalog::for_project`]): what the
//! project runs is the live set — each extension's active deployment in the
//! store (`extension_store`), a bundled one without a deployment as it
//! ships — whatever root a caller names. A source edit changes nothing here
//! until it is deployed. The load is cached behind a key of the live set,
//! the extension settings and the source folders present.
//!
//! **Source** (standalone, [`ExtensionCatalog::new`]: the CLI, the SDK's
//! checks): each root is its own project and its extensions load from
//! their sources, cached behind a **stat-only fingerprint** of every source
//! folder and both settings files (paths, sizes, mtimes): a hit walks the
//! folders with `stat` and parses nothing.
//!
//! **The change signal** ([`ExtensionCatalog::changes`]) says what the
//! project runs may have changed: the `extensions` setting, from its
//! `config.changed` reactor. A deploy activates directly.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oxplow_domain::DomainError;
use parking_lot::Mutex;

use crate::extensions::{self, Deployment, Extension};

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
    /// back the others; oxplow's own required one too. Returns the held
    /// extensions and why.
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
            if (!extension.enabled && !extension.public_contracts.blocked) || can_load(extension) {
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
            let absent = |r: &String| !present.contains(r);
            let missing = admitted.iter().position(|e| {
                e.enabled
                    && (e.public_contracts.imports.iter().any(|import| {
                        !import.optional && !present.contains(&import.contribution.extension)
                    }) || e.requires.iter().any(absent))
            });
            let Some(index) = missing else { break };
            let out = admitted.remove(index);
            let why = held.entry(out.name.clone()).or_default();
            for r in out.requires.iter().filter(|r| absent(r)) {
                why.push(format!("it requires `{r}`, which isn't active"));
            }
            let owners: Vec<String> = out
                .public_contracts
                .imports
                .iter()
                .filter(|i| !i.optional && !present.contains(&i.contribution.extension))
                .map(|i| format!("`{}`", i.contribution.extension))
                .collect();
            if !owners.is_empty() {
                why.push(format!(
                    "it imports from {}, which isn't active",
                    owners.join(", ")
                ));
            }
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

    /// Forget `name`'s failures: it runs something else now (the copy
    /// that ships, after its deployment gave way).
    pub(crate) fn forget(&mut self, name: &str) {
        self.0.remove(name);
    }

    /// Hold the extensions `problems` names, if each is an enabled
    /// extension of `snapshot` that can still be held (a
    /// second failure takes its last working version out too). False when
    /// the problems aren't one extension's to hold: the candidate fails.
    pub(crate) fn hold(
        &mut self,
        snapshot: &CatalogSnapshot,
        problems: &BTreeMap<String, Vec<String>>,
    ) -> bool {
        let holdable = |name: &String| {
            snapshot
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

/// A change to the live set a candidate activates: `name → Some(hash)` runs
/// that deployment, `name → None` runs none (a bundled one as it ships).
pub type LiveChanges = BTreeMap<String, Option<String>>;

/// Clears an activation's staged changes when it ends.
pub(crate) struct PendingChanges(Arc<Mutex<LiveChanges>>);

impl Drop for PendingChanges {
    fn drop(&mut self) {
        self.0.lock().clear();
    }
}

pub struct ExtensionCatalog {
    /// The project whose deployments it loads (the daemon's main checkout).
    /// `None` standalone (the CLI, the SDK's checks): each root is its own
    /// project, loaded from its sources.
    project: Option<PathBuf>,
    by_root: Mutex<HashMap<PathBuf, Entry>>,
    /// The deployed load and its key.
    deployed: Mutex<Option<(String, Arc<CatalogSnapshot>)>>,
    /// Each extension's active deployment (`name → hash`).
    live: Mutex<BTreeMap<String, String>>,
    /// The changes to it an activation is staging: part of what
    /// [`Self::snapshot`] loads until that activation ends.
    pending: Arc<Mutex<LiveChanges>>,
    /// Last atomically published primary-project declarations. Raw loads stay
    /// available for authoring and candidate validation.
    active: Mutex<Option<Arc<CatalogSnapshot>>>,
    /// Full loads performed; tests read it to prove a hit parses nothing.
    loads: AtomicUsize,
    /// The primary worktree's extensions may have changed.
    changes: tokio::sync::broadcast::Sender<()>,
    runtime_changes: tokio::sync::broadcast::Sender<()>,
    /// oxplow's own extensions run from their sources
    /// (`enableBundledExtensions: false` in a dev build), as read when it
    /// was made: changing it takes a restart, since flipping it under a
    /// running foundation would take foundation away.
    bundled_from_sources: bool,
    /// A private catalog holding one candidate for staging: it is no
    /// generation a reader could straddle, so reading it needs no pin. Only
    /// the pin assertion reads it.
    #[cfg(test)]
    staged: bool,
}

impl Default for ExtensionCatalog {
    fn default() -> Self {
        Self {
            project: None,
            by_root: Mutex::default(),
            deployed: Mutex::default(),
            live: Mutex::default(),
            pending: Arc::default(),
            active: Mutex::default(),
            loads: AtomicUsize::default(),
            changes: tokio::sync::broadcast::channel(64).0,
            runtime_changes: tokio::sync::broadcast::channel(64).0,
            bundled_from_sources: false,
            #[cfg(test)]
            staged: false,
        }
    }
}

impl ExtensionCatalog {
    /// Standalone: each root it reads is its own project.
    pub fn new() -> Self {
        Self::default()
    }

    /// The daemon's: whatever root it is asked about, it loads
    /// `project_dir`'s live set.
    pub fn for_project(project_dir: &Path) -> Self {
        let settings = oxplow_config::extension_settings(project_dir);
        if settings.bundled_requested == Some(false) && !settings.bundled_from_sources {
            tracing::warn!(
                "enableBundledExtensions: false is honored only by a dev build; \
                 oxplow's own extensions run as they ship"
            );
        }
        Self {
            project: Some(project_dir.to_path_buf()),
            bundled_from_sources: settings.bundled_from_sources,
            ..Self::default()
        }
    }

    /// `project`'s extension settings, with where oxplow's own extensions
    /// come from as it was when this catalog was made.
    pub fn settings(&self, project: &Path) -> oxplow_config::ExtensionSettings {
        oxplow_config::ExtensionSettings {
            bundled_from_sources: self.bundled_from_sources,
            ..oxplow_config::extension_settings(project)
        }
    }

    /// A private catalog whose only generation is `candidate`, for staging
    /// it before it is published.
    pub(crate) fn staged(project_dir: &Path, candidate: Arc<CatalogSnapshot>) -> Self {
        let catalog = Self {
            #[cfg(test)]
            staged: true,
            ..Self::for_project(project_dir)
        };
        catalog.publish_active(candidate);
        catalog
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

    /// Each extension's active deployment (`name → hash`).
    pub fn live(&self) -> BTreeMap<String, String> {
        self.live.lock().clone()
    }

    /// Set the live set: at startup from the active deployments recorded
    /// for the project, and after each full publication from what it
    /// published (a core-only baseline leaves it as it is).
    pub(crate) fn set_live(&self, live: BTreeMap<String, String>) {
        *self.live.lock() = live;
    }

    /// Run `name` as it ships rather than as deployed: out of the live set
    /// and out of the changes being staged.
    pub(crate) fn give_way(&self, name: &str) {
        self.live.lock().remove(name);
        self.pending.lock().remove(name);
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

    /// Say they may have: the `extensions` setting changed.
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
        match &self.project {
            Some(project) => self.deployed_snapshot(project),
            None => self.source_snapshot(root),
        }
    }

    /// Stage `changes` to the live set: [`Self::snapshot`] loads them until
    /// the returned guard drops (the activation that stages them ends).
    pub(crate) fn stage_changes(&self, changes: &LiveChanges) -> PendingChanges {
        *self.pending.lock() = changes.clone();
        PendingChanges(self.pending.clone())
    }

    /// The project's live set with the staged changes applied, loaded.
    fn deployed_snapshot(&self, project: &Path) -> Arc<CatalogSnapshot> {
        let settings = self.settings(project);
        let mut live = self.live.lock().clone();
        for (name, hash) in self.pending.lock().iter() {
            match hash {
                Some(hash) => live.insert(name.clone(), hash.clone()),
                None => live.remove(name),
            };
        }
        let names = extensions::known_names(project, &settings);
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(format!("{settings:?}|{live:?}|{names:?}").as_bytes());
        let key = format!("{:016x}", h.digest());
        let mut cache = self.deployed.lock();
        if let Some((cached, snapshot)) = cache.as_ref() {
            if *cached == key {
                return snapshot.clone();
            }
        }
        let snapshot = Arc::new(CatalogSnapshot {
            extensions: Arc::new(extensions::load_deployed(project, &settings, &live)),
            generation: key.clone(),
            source_generation: key.clone(),
        });
        self.loads.fetch_add(1, Ordering::Relaxed);
        *cache = Some((key, snapshot.clone()));
        snapshot
    }

    fn source_snapshot(&self, root: &Path) -> Arc<CatalogSnapshot> {
        let fingerprint = fingerprint(root);
        let mut cache = self.by_root.lock();
        if let Some(entry) = cache.get(root) {
            if entry.fingerprint == fingerprint {
                return entry.snapshot.clone();
            }
        }
        let extensions = Arc::new(extensions::load_extensions(root));
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
    ///
    /// A reader that combines this with another generation-owned map
    /// (commands, capabilities, providers, models) must hold a generation
    /// pin (`ExtensionActivation::read`, or a run's pin): tests assert one is
    /// in scope. A revocation gate that has to answer even while a
    /// publication waits uses [`Self::live_snapshot`].
    pub fn runtime_snapshot(&self, root: &Path) -> Arc<CatalogSnapshot> {
        #[cfg(test)]
        assert!(
            self.staged
                || self.active.lock().is_none()
                || crate::extension_activation::ExtensionActivation::reader_in_scope(),
            "a generation read outside a generation pin: pin it (`ExtensionActivation::read`), \
             or read it explicitly (`unpinned`, `live_snapshot`)\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        self.live_snapshot(root)
    }

    /// The declarations as one value, read by a caller that needs nothing
    /// else of the generation: everything it uses comes from the returned
    /// snapshot, and what it starts (a command, a run) pins for itself.
    /// `reason` says why that holds, so each such reader is explicit.
    pub fn unpinned(&self, root: &Path, _reason: &'static str) -> Arc<Vec<Extension>> {
        self.live_snapshot(root).extensions.clone()
    }

    /// The same declarations as [`Self::runtime_snapshot`], read as a
    /// revocation gate: whether an owner is still enabled and current
    /// right now, to refuse a callback, a job's publication or a run built
    /// against a generation that has since moved. Such a check must not
    /// wait on a publication (which waits on its readers), so it holds no
    /// pin and decides nothing about which generation other maps are at.
    pub fn live_snapshot(&self, root: &Path) -> Arc<CatalogSnapshot> {
        let Some(project) = self.project.as_deref() else {
            return self.snapshot(root);
        };
        let Some(active) = self.active.lock().clone() else {
            return self.snapshot(root);
        };
        let settings = self.settings(project);
        if !active
            .extensions()
            .iter()
            .any(|extension| extension.enabled && extensions::is_off(&settings, &extension.name))
        {
            return active;
        }
        Arc::new(CatalogSnapshot {
            extensions: Arc::new(extensions::resolve_catalog(
                active.extensions().as_ref().clone(),
                &settings,
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
        match &self.project {
            Some(_) => self.snapshot(root).generation().to_string(),
            None => fingerprint_generation(&fingerprint(root)),
        }
    }

    /// One extension by name. A disabled one is an error saying so; an
    /// unknown one is `NotFound`.
    pub fn named(&self, root: &Path, name: &str) -> Result<Extension, DomainError> {
        let all = self.unpinned(root, "a lookup by name reads one snapshot");
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

    /// One page by `<extension>/<slug>`.
    pub fn find_page(
        &self,
        root: &Path,
        id: &str,
    ) -> Result<crate::pages::schema::Page, DomainError> {
        let (ext, slug) = id.split_once('/').ok_or(DomainError::NotFound)?;
        self.named(root, ext)?
            .page_files
            .into_iter()
            .find(|p| p.slug == slug)
            .ok_or(DomainError::NotFound)
    }

    /// The page a page URI's `page` param names: `<extension>/<slug>`, or a
    /// bare slug for one of oxplow's own extensions (core-named, as
    /// `oxplow:oxplow/page?page=tasks`).
    pub fn page_named(
        &self,
        root: &Path,
        page: &str,
    ) -> Result<crate::pages::schema::Page, DomainError> {
        if page.contains('/') {
            return self.find_page(root, page);
        }
        self.get(root)
            .iter()
            .filter(|e| e.enabled && e.namespace == crate::extensions::CORE_NAMESPACE)
            .flat_map(|e| e.page_files.iter())
            .find(|p| p.slug == page)
            .cloned()
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

/// The active deployments of a published load (`name → hash`).
pub(crate) fn live_of(extensions: &[Extension]) -> BTreeMap<String, String> {
    extensions
        .iter()
        .filter_map(|e| match &e.deployment {
            Deployment::Deployed { hash } => Some((e.name.clone(), hash.clone())),
            _ => None,
        })
        .collect()
}

/// What a source load of `root` depends on: every file of every source
/// folder its settings know, and both settings files.
fn fingerprint(root: &Path) -> Fingerprint {
    let settings = oxplow_config::extension_settings(root);
    let mut out = Fingerprint::new();
    walk(&root.join(extensions::EXTENSIONS_DIR), &mut out, true);
    for name in extensions::known_names(root, &settings) {
        let dir = settings.source(root, &name);
        if !dir.starts_with(root.join(extensions::EXTENSIONS_DIR)) {
            walk(&dir, &mut out, false);
        }
    }
    push_file(&oxplow_config::config_path(root), &mut out);
    push_file(&oxplow_config::personal_path(root), &mut out);
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
    const PAGE: &str =
        "title: Tasks\ndata:\n  items: { sql: SELECT uri FROM v_work_item }\nview: { viz: table, from: items }\n";

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
            ".oxplow/extensions/good/extension.yaml",
            &manifest("good", "one"),
        );
        write(
            dir.path(),
            ".oxplow/extensions/edited/extension.yaml",
            &manifest("edited", "one"),
        );
        let catalog = ExtensionCatalog::new();
        let (first, held) = catalog.snapshot(dir.path()).admitted(None, &Failed::new());
        assert!(held.is_empty(), "{held:?}");
        assert_eq!(names(&first), ["edited:one", "good:one"]);

        // `edited` breaks, `good` changes, `fresh` arrives broken.
        write(
            dir.path(),
            ".oxplow/extensions/edited/extension.yaml",
            &format!("{}bogus: 1\n", manifest("edited", "two")),
        );
        write(
            dir.path(),
            ".oxplow/extensions/good/extension.yaml",
            &manifest("good", "two"),
        );
        write(
            dir.path(),
            ".oxplow/extensions/fresh/extension.yaml",
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

    /// oxplow's own required extension is held like any other: a broken
    /// one keeps its last working version, or is out when it has none.
    #[test]
    fn a_required_extension_that_cant_load_is_held_like_any_other() {
        let dir = tempfile::tempdir().unwrap();
        let foundation = crate::bundled_extensions::find("oxplow-foundation").unwrap();
        let to = dir.path().join(".oxplow/extensions/oxplow-foundation");
        for (rel, bytes) in foundation.files {
            let file = to.join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let catalog = ExtensionCatalog::new();
        let (good, held) = catalog.snapshot(dir.path()).admitted(None, &Failed::new());
        assert!(held.is_empty(), "{held:?}");
        let manifest = std::fs::read_to_string(to.join("extension.yaml")).unwrap();
        std::fs::write(to.join("extension.yaml"), format!("{manifest}bogus: 1\n")).unwrap();
        let broken = catalog.snapshot(dir.path());
        let (kept, held) = broken.admitted(Some(&good), &Failed::new());
        assert!(
            held["oxplow-foundation"]
                .iter()
                .any(|e| e.contains("last working version stays active")),
            "{held:?}"
        );
        let running = kept
            .extensions()
            .iter()
            .find(|e| e.name == "oxplow-foundation")
            .unwrap();
        assert!(running.errors.is_empty());
        let (out, held) = broken.admitted(None, &Failed::new());
        assert!(held.contains_key("oxplow-foundation"));
        assert!(!out
            .extensions()
            .iter()
            .any(|e| e.name == "oxplow-foundation"));
        // A preparation failure holds it too.
        let mut failed = Failed::new();
        assert!(failed.hold(
            &good,
            &BTreeMap::from([("oxplow-foundation".to_string(), vec!["no".to_string()])])
        ));
    }

    /// An extension whose requirement isn't active is held with it, and
    /// so is one that requires that one in turn.
    #[test]
    fn an_extension_whose_requirement_is_off_is_held_at_activation() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = |name: &str, extra: &str| {
            format!("manifest: 2\nname: {name}\ndescription: d\n{extra}intent:\n  purpose: x\n  examples: [{{ name: a }}]\n")
        };
        for (name, extra) in [
            ("alone", ""),
            ("needy", "requires: [ghost]\n"),
            ("chained", "requires: [needy]\n"),
            ("fine", "requires: [alone]\n"),
        ] {
            write(
                dir.path(),
                &format!(".oxplow/extensions/{name}/extension.yaml"),
                &manifest(name, extra),
            );
        }
        let catalog = ExtensionCatalog::new();
        let (admitted, held) = catalog.snapshot(dir.path()).admitted(None, &Failed::new());
        assert_eq!(names(&admitted), ["alone:d", "fine:d"]);
        assert_eq!(held.keys().collect::<Vec<_>>(), ["chained", "needy"]);
        assert!(
            held["needy"]
                .iter()
                .any(|e| e == "it requires `ghost`, which isn't active"),
            "{held:?}"
        );
        assert!(
            held["chained"]
                .iter()
                .any(|e| e == "it requires `needy`, which isn't active"),
            "{held:?}"
        );
    }

    #[test]
    fn a_hit_parses_nothing_and_an_edit_misses() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/tasks.yaml",
            PAGE,
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
            catalog.find_page(dir.path(), "review/tasks").unwrap().title,
            "Tasks"
        );
        assert_eq!(catalog.loads(), 1);

        // A page edit is seen on the very next call.
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/tasks.yaml",
            "title: Tasks by status\ndata:\n  items: { sql: SELECT uri FROM v_work_item }\nview: { viz: table, from: items }\n",
        );
        assert_eq!(
            catalog.find_page(dir.path(), "review/tasks").unwrap().title,
            "Tasks by status"
        );
        assert_eq!(catalog.loads(), 2);
        // So is a new page file and a removed one.
        write(
            dir.path(),
            ".oxplow/extensions/review/pages/more.yaml",
            PAGE,
        );
        assert!(catalog.find_page(dir.path(), "review/more").is_ok());
        std::fs::remove_file(dir.path().join(".oxplow/extensions/review/pages/more.yaml")).unwrap();
        assert!(matches!(
            catalog.find_page(dir.path(), "review/more"),
            Err(DomainError::NotFound)
        ));
        assert_eq!(catalog.loads(), 4);
    }

    /// The daemon's catalog runs what is deployed in the project, whichever
    /// root is asked about: a stream's working copy isn't read, and the
    /// main worktree's settings say what is on.
    #[test]
    fn a_projects_catalog_takes_everything_from_the_project_for_every_root() {
        let main = tempfile::tempdir().unwrap();
        let side = tempfile::tempdir().unwrap();
        write(side.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            side.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [oxplow-bundled]\n",
        );
        let catalog = ExtensionCatalog::for_project(main.path());
        assert!(matches!(
            catalog.named(side.path(), "review"),
            Err(DomainError::NotFound)
        ));
        assert!(
            catalog.named(side.path(), "oxplow-bundled").is_ok(),
            "the working copy's own personal.yaml doesn't count"
        );
        write(
            main.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [oxplow-bundled]\n",
        );
        let err = catalog.named(side.path(), "oxplow-bundled").unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");
        assert_eq!(catalog.project_of(side.path()), main.path());
    }

    #[test]
    fn roots_are_cached_separately_and_project_config_counts() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path(), ".oxplow/extensions/review/extension.yaml", EXT);
        write(
            b.path(),
            ".oxplow/extensions/board/extension.yaml",
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
            ".oxplow/personal.yaml",
            "disableExtensions: [review]\n",
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
