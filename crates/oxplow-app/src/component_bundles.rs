//! What a custom component's frame runs (tsk984): its bundle, keyed by its
//! **version** — the component's approval hash over the bundle's files and
//! the commands it may run
//! ([`crate::exec_consent::ProjectProgram::component_hash`]). The daemon
//! serves the frame at `/components/v/<version>/…` from the bundle's folder
//! in its deployment on disk: deployments are immutable, so what is served
//! is what was hashed, and the frame's `invoke` names the version it was
//! loaded at, which must be the approved one. Nothing holds a bundle's
//! bytes: an open frame on an older version loads from that deployment
//! while the store keeps it, and once it's pruned the frame must reload.
//!
//! The folder is read as it is served — the extension's path joined with
//! the manifest's `bundle` — so a spelling that differs from the disk's
//! only in case still hashes every file it serves.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::DomainError;

use crate::exec_consent::component_program;
use crate::extension_catalog::CatalogSnapshot;
use crate::extensions::custom_components::CustomComponent;
use crate::extensions::Extension;
use crate::vocabulary_reactor::VocabularyErrors;

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
    /// The bundle's folder (in its deployment).
    dir: PathBuf,
    /// Every file of the folder, by `/`-separated path inside it: all a
    /// frame may load.
    files: BTreeSet<String>,
}

impl BundleSnapshot {
    /// The file at `rel` (`""` is `index.html`), read from the bundle's
    /// folder; `None` for anything that isn't exactly one of its files, or
    /// once its deployment is gone.
    pub fn file(&self, rel: &str) -> Option<Vec<u8>> {
        let rel = if rel.is_empty() { "index.html" } else { rel };
        if !self.files.contains(rel) {
            return None;
        }
        std::fs::read(self.dir.join(rel)).ok()
    }
}

/// The loaded bundles.
#[derive(Default)]
pub struct ComponentBundles {
    /// Every bundle loaded in this process, by version: a folder and a list
    /// of names each, so none is ever dropped for an open frame's sake.
    loaded: parking_lot::Mutex<HashMap<String, Arc<BundleSnapshot>>>,
    active: parking_lot::Mutex<Option<PreparedComponentBundles>>,
}

/// Bundles captured from one enabled catalog generation. Preparation hashes
/// each bundle's files; it does not mutate the live bundle map or execute
/// component code.
pub struct PreparedComponentBundles {
    root: PathBuf,
    generation: String,
    bundles: BTreeMap<(String, String), Arc<BundleSnapshot>>,
}

impl PreparedComponentBundles {
    pub fn prepare(root: &Path, snapshot: &CatalogSnapshot) -> Result<Self, VocabularyErrors> {
        let mut bundles = BTreeMap::new();
        let mut errors = VocabularyErrors::new();
        for ext in snapshot.extensions().iter().filter(|ext| ext.enabled) {
            for component in &ext.custom_components {
                let key = (ext.name.clone(), component.id.clone());
                if bundles.contains_key(&key) {
                    add_error(
                        &mut errors,
                        &ext.name,
                        "custom component bundle identity is duplicated",
                    );
                    continue;
                }
                match build_snapshot(
                    root,
                    ext,
                    component,
                    Some(snapshot.generation().to_string()),
                ) {
                    Ok(bundle) => {
                        bundles.insert(key, bundle);
                    }
                    Err(_) => add_error(
                        &mut errors,
                        &ext.name,
                        "custom component bundle could not be staged",
                    ),
                }
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
        let mut loaded = registry.loaded.lock();
        for bundle in self.bundles.values() {
            loaded.insert(bundle.version.clone(), bundle.clone());
        }
        drop(loaded);
        *registry.active.lock() = Some(self);
    }
}

impl ComponentBundles {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bundle of `component` of `ext` under `root` (the worktree its
    /// lens is shown in): the active generation's, or for another root
    /// read as it is now.
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
        let snapshot = match active {
            Some(active) => active.ok_or(DomainError::NotFound)?,
            None => build_snapshot(root, ext, component, None)?,
        };
        self.loaded
            .lock()
            .insert(snapshot.version.clone(), snapshot.clone());
        Ok(snapshot)
    }

    /// The bundle loaded at `version`, if one was.
    pub fn get(&self, version: &str) -> Option<Arc<BundleSnapshot>> {
        self.loaded.lock().get(version).cloned()
    }

    /// Old frames can still load their files, but invoking against a later
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

/// `component`'s bundle folder under `root`: its file names and its
/// version, hashed from the files on disk. A symlink in it is an error
/// (`Disk::paths`).
fn build_snapshot(
    root: &Path,
    ext: &Extension,
    component: &CustomComponent,
    generation: Option<String>,
) -> Result<Arc<BundleSnapshot>, DomainError> {
    let program = component_program(ext, component);
    let invalid = |e: std::io::Error| DomainError::Invalid(format!("{}: {e}", program.program));
    let source = crate::extensions::files_at(root, &program.program).map_err(invalid)?;
    let files: BTreeSet<String> = source.paths().map_err(invalid)?.into_iter().collect();
    if !files.contains("index.html") {
        return Err(DomainError::Invalid(format!(
            "{}: no `index.html`",
            program.program
        )));
    }
    let version = program.component_hash(source.as_ref()).map_err(invalid)?;
    Ok(Arc::new(BundleSnapshot {
        version,
        extension: ext.name.clone(),
        component: component.id.clone(),
        generation,
        commands: component.commands.clone(),
        assets: component.assets.clone(),
        dir: root.join(&program.program),
        files,
    }))
}

fn add_error(errors: &mut VocabularyErrors, owner: &str, message: &str) {
    let owner_errors = errors.entry(owner.to_string()).or_default();
    if !owner_errors.iter().any(|error| error == message) {
        owner_errors.push(message.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// oxplow-bundled's history graph is a component like a project's:
    /// its bundle is read from its shipped deployment on disk, hashed by
    /// the one approval hash, staged by activation's preparation and served
    /// from that folder.
    #[test]
    fn a_bundled_component_is_staged_and_served_from_its_shipped_deployment() {
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
