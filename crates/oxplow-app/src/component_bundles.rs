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

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use oxplow_domain::DomainError;

use crate::exec_consent::{component_program, ProjectProgram};
use crate::extensions::custom_components::{
    BundleLook, CustomComponent, MAX_BUNDLE_BYTES, MAX_BUNDLE_FILES,
};
use crate::extensions::{Extension, ExtensionFiles};

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
    /// The commands it may run, as hashed.
    pub commands: Vec<String>,
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
        if ext.origin == "bundled" {
            return Err(DomainError::Invalid(format!(
                "`{}` comes with oxplow, and its components aren't served",
                ext.name
            )));
        }
        let program = component_program(ext, component);
        let files = read_bundle(root, &program)?;
        let version = program
            .component_hash(&files)
            .map_err(|e| DomainError::Invalid(format!("{}: {e}", program.program)))?;
        let snapshot = Arc::new(BundleSnapshot {
            version,
            extension: ext.name.clone(),
            component: component.id.clone(),
            commands: component.commands.clone(),
            files: files.0,
        });
        let mut kept = self.kept.lock();
        kept.retain(|s| s.version != snapshot.version);
        kept.push_back(snapshot.clone());
        while kept.len() > KEPT {
            kept.pop_front();
        }
        Ok(snapshot)
    }

    /// The loaded bundle at `version`, if it is still kept.
    pub fn get(&self, version: &str) -> Option<Arc<BundleSnapshot>> {
        self.kept
            .lock()
            .iter()
            .find(|s| s.version == version)
            .cloned()
    }
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
