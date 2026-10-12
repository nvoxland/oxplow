//! What a custom component's frame runs (tsk984): its bundle, read once
//! into a snapshot keyed by its **version** — the component's approval hash
//! over exactly those files and the commands it may run
//! ([`crate::exec_consent::ProjectProgram::component_hash`]). The daemon
//! serves the frame from the snapshot alone (`/components/v/<version>/…`),
//! and the frame's `invoke` names the version it was loaded at, which must
//! be the approved one. So what runs is what was hashed: a bundle edited
//! on disk after loading changes nothing the frame runs, and one loaded
//! while edited can't act on the approval of what the disk says later.
//!
//! The folder is read as the daemon used to serve it — the extension's
//! path joined with the manifest's `bundle` — so a spelling that differs
//! from the disk's only in case still hashes every file it serves.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::DomainError;

use crate::exec_consent::{component_program, ProjectProgram};
use crate::extension_catalog::CatalogSnapshot;
use crate::extensions::custom_components::{
    BundleLook, CustomComponent, MAX_BUNDLE_BYTES, MAX_BUNDLE_FILES,
};
use crate::extensions::{Extension, ExtensionFiles};
use crate::vocabulary_reactor::VocabularyErrors;

const MAX_GENERATION_BUNDLES: usize = 64;
const MAX_GENERATION_BYTES: u64 = 128 * 1024 * 1024;

/// How many loaded bundles are kept for their frames; an older one's frame
/// is asked to reload.
const KEPT: usize = 16;

/// One component's bundle as it was loaded.
#[derive(Debug)]
pub struct BundleSnapshot {
    /// Its approval hash: what an approval of it names.
    pub version: String,
    /// The extension and the component it is.
    pub extension: String,
    pub component: String,
    /// Catalog generation whose enabled declarations produced this snapshot.
    pub(crate) generation: Option<String>,
    /// The commands it may run, as hashed.
    pub commands: Vec<String>,
    pub assets: Vec<String>,
    /// Every file of the folder, by `/`-separated path inside it.
    files: BTreeMap<String, Vec<u8>>,
}

impl BundleSnapshot {
    /// The file at `rel` (`""` is `index.html`); `None` for anything that
    /// isn't exactly one of its files.
    pub fn file(&self, rel: &str) -> Option<&[u8]> {
        let rel = if rel.is_empty() { "index.html" } else { rel };
        self.files.get(rel).map(Vec::as_slice)
    }
}

/// The loaded bundles, newest last.
#[derive(Default)]
pub struct ComponentBundles {
    kept: parking_lot::Mutex<VecDeque<Arc<BundleSnapshot>>>,
    active: parking_lot::Mutex<Option<PreparedComponentBundles>>,
}

/// Bundles captured from one enabled catalog generation. Preparation reads
/// at most 64 bundles and 128 MiB of aggregate file bytes; it does not mutate
/// the live bundle map or execute component code.
pub struct PreparedComponentBundles {
    root: PathBuf,
    generation: String,
    bundles: BTreeMap<(String, String), Arc<BundleSnapshot>>,
}

impl PreparedComponentBundles {
    pub fn prepare(root: &Path, snapshot: &CatalogSnapshot) -> Result<Self, VocabularyErrors> {
        let mut bundles = BTreeMap::new();
        let mut keys = BTreeSet::new();
        let mut errors = VocabularyErrors::new();
        let mut count = 0usize;
        let mut total_bytes = 0u64;

        for ext in snapshot.extensions().iter().filter(|ext| ext.enabled) {
            for component in &ext.custom_components {
                count += 1;
                if count > MAX_GENERATION_BUNDLES {
                    return Err(host_error("too many custom component bundles"));
                }
                let key = (ext.name.clone(), component.id.clone());
                if !keys.insert(key.clone()) {
                    add_error(
                        &mut errors,
                        &ext.name,
                        "custom component bundle identity is duplicated",
                    );
                    continue;
                }
                let bundle = match build_snapshot(
                    root,
                    ext,
                    component,
                    Some(snapshot.generation().to_string()),
                ) {
                    Ok(bundle) => bundle,
                    Err(_) => {
                        add_error(
                            &mut errors,
                            &ext.name,
                            "custom component bundle could not be staged",
                        );
                        continue;
                    }
                };
                let bytes = bundle
                    .files
                    .values()
                    .map(|file| file.len() as u64)
                    .sum::<u64>();
                if total_bytes.saturating_add(bytes) > MAX_GENERATION_BYTES {
                    add_error(
                        &mut errors,
                        &ext.name,
                        "custom component generation exceeds its byte limit",
                    );
                    continue;
                }
                total_bytes += bytes;
                bundles.insert(key, bundle);
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self {
            root: root.to_path_buf(),
            generation: snapshot.generation().to_string(),
            bundles,
        })
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Replace the active generation's bundle map in one infallible swap.
    pub fn publish(self, registry: &ComponentBundles) {
        *registry.active.lock() = Some(self);
    }
}

impl ComponentBundles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read `component` of `ext` under `root` (the worktree its lens is
    /// shown in) as it is now, and keep it: its snapshot.
    pub fn load(
        &self,
        root: &Path,
        ext: &Extension,
        component: &CustomComponent,
    ) -> Result<Arc<BundleSnapshot>, DomainError> {
        let active = {
            let active = self.active.lock();
            active.as_ref().and_then(|active| {
                (active.root.as_path() == root).then(|| {
                    active
                        .bundles
                        .get(&(ext.name.clone(), component.id.clone()))
                        .cloned()
                })
            })
        };
        if let Some(active) = active {
            let snapshot = active.ok_or(DomainError::NotFound)?;
            self.keep(snapshot.clone());
            return Ok(snapshot);
        }
        let snapshot = build_snapshot(root, ext, component, None)?;
        self.keep(snapshot.clone());
        Ok(snapshot)
    }

    fn keep(&self, snapshot: Arc<BundleSnapshot>) {
        let mut kept = self.kept.lock();
        kept.retain(|s| s.version != snapshot.version);
        kept.push_back(snapshot);
        while kept.len() > KEPT {
            kept.pop_front();
        }
    }

    /// The loaded bundle at `version`, if it is still kept.
    pub fn get(&self, version: &str) -> Option<Arc<BundleSnapshot>> {
        let active = {
            let active = self.active.lock();
            active.as_ref().and_then(|active| {
                active
                    .bundles
                    .values()
                    .find(|snapshot| snapshot.version == version)
                    .cloned()
            })
        };
        active.or_else(|| {
            self.kept
                .lock()
                .iter()
                .find(|s| s.version == version)
                .cloned()
        })
    }

    /// Old frames can still read retained bytes, but invoking against a later
    /// activated catalog requires reloading. A bundle of another root (an
    /// authoring root, not the activated project) has no generation to
    /// compare, so its frames stay current; before any activation nothing
    /// is current.
    pub fn is_current(&self, root: &Path, bundle: &BundleSnapshot) -> bool {
        let active = self.active.lock();
        match active.as_ref() {
            Some(active) if active.root == root => {
                bundle.generation.as_deref() == Some(active.generation())
            }
            Some(_) => true,
            None => false,
        }
    }
}

fn build_snapshot(
    root: &Path,
    ext: &Extension,
    component: &CustomComponent,
    generation: Option<String>,
) -> Result<Arc<BundleSnapshot>, DomainError> {
    let program = component_program(ext, component);
    let files = read_bundle(root, &program)?;
    let version = program
        .component_hash(&files)
        .map_err(|e| DomainError::Invalid(format!("{}: {e}", program.program)))?;
    Ok(Arc::new(BundleSnapshot {
        version,
        extension: ext.name.clone(),
        component: component.id.clone(),
        generation,
        commands: component.commands.clone(),
        assets: component.assets.clone(),
        files: files.0,
    }))
}

fn add_error(errors: &mut VocabularyErrors, owner: &str, message: &str) {
    let owner_errors = errors.entry(owner.to_string()).or_default();
    if !owner_errors.iter().any(|error| error == message) {
        owner_errors.push(message.to_string());
    }
}

fn host_error(message: &str) -> VocabularyErrors {
    VocabularyErrors::from([("<host>".to_string(), vec![message.to_string()])])
}

/// A bundle's files in memory, bytes and all.
pub(crate) struct BundleFiles(BTreeMap<String, Vec<u8>>);

impl ExtensionFiles for BundleFiles {
    fn read(&self, rel: &str) -> Option<String> {
        self.0
            .get(rel)
            .and_then(|b| String::from_utf8(b.clone()).ok())
    }
    fn paths(&self) -> std::io::Result<Vec<String>> {
        Ok(self.0.keys().cloned().collect())
    }
    fn bytes(&self, rel: &str) -> std::io::Result<Vec<u8>> {
        self.0
            .get(rel)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel.to_string()))
    }
    fn list(&self, _dir: &str) -> Vec<String> {
        Vec::new()
    }
    fn bundle_stat(&self, _rel: &str) -> BundleLook {
        BundleLook::Unknown
    }
}

/// Every file of `program`'s bundle folder under `root`, within the
/// bundle caps. A symlink in it is an error (`Disk::paths`).
pub(crate) fn read_bundle(
    root: &Path,
    program: &ProjectProgram,
) -> Result<BundleFiles, DomainError> {
    let invalid = |e: std::io::Error| DomainError::Invalid(format!("{}: {e}", program.program));
    let source = crate::extensions::files_at(root, &program.program).map_err(invalid)?;
    let paths = source.paths().map_err(invalid)?;
    if paths.len() > MAX_BUNDLE_FILES {
        return Err(DomainError::Invalid(format!(
            "{}: more than {MAX_BUNDLE_FILES} files",
            program.program
        )));
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for rel in paths {
        let bytes = source.bytes(&rel).map_err(invalid)?;
        total += bytes.len() as u64;
        if total > MAX_BUNDLE_BYTES {
            return Err(DomainError::Invalid(format!(
                "{}: more than {MAX_BUNDLE_BYTES} bytes",
                program.program
            )));
        }
        files.insert(rel, bytes);
    }
    if !files.contains_key("index.html") {
        return Err(DomainError::Invalid(format!(
            "{}: no `index.html`",
            program.program
        )));
    }
    Ok(BundleFiles(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// oxplow-bundled's history graph is a component like a project's:
    /// its bundle is read from the embedded files, hashed by the one
    /// approval hash, staged by activation's preparation and served from
    /// the snapshot.
    #[test]
    fn a_bundled_component_is_staged_and_served_from_its_embedded_files() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = crate::extension_catalog::ExtensionCatalog::new();
        let snapshot = catalog.snapshot(dir.path());
        let ext = snapshot
            .extensions()
            .iter()
            .find(|e| e.name == "oxplow-bundled")
            .expect("bundled")
            .clone();
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let component = ext
            .custom_components
            .iter()
            .find(|c| c.id == "history-graph")
            .expect("declared")
            .clone();

        let bundle = build_snapshot(dir.path(), &ext, &component, None).expect("built");
        let shipped = crate::bundled_extensions::find("oxplow-bundled").unwrap();
        assert_eq!(
            bundle.file("").unwrap(),
            shipped
                .bytes("components/history-graph/index.html")
                .unwrap()
        );
        assert_eq!(
            bundle.file("app.js").unwrap(),
            shipped.bytes("components/history-graph/app.js").unwrap()
        );
        let program = component_program(&ext, &component);
        assert_eq!(program.hash(dir.path()).unwrap(), bundle.version);

        let prepared = PreparedComponentBundles::prepare(dir.path(), &snapshot).expect("staged");
        let registry = ComponentBundles::new();
        prepared.publish(&registry);
        let loaded = registry.load(dir.path(), &ext, &component).expect("loaded");
        assert_eq!(loaded.version, bundle.version);
        assert!(registry.get(&bundle.version).is_some());
    }
}
