//! Candidate catalogs for authoring checks, and bounded install recovery.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::Extension;

/// Held while an install, update or startup recovery decides and publishes:
/// an exclusive lock on `.oxplow/install.lock` in the project, so it
/// serializes across threads and across processes sharing the project.
/// Released when dropped (or when the process dies).
#[must_use = "the lock is released when this is dropped"]
pub(super) struct InstallLock {
    _file: std::fs::File,
}

impl InstallLock {
    /// Wait for the project's install lock.
    pub(super) fn acquire(root: &Path) -> std::io::Result<Self> {
        let dir = root.join(".oxplow");
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("install.lock"))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

const STAGE_PREFIX: &str = ".ext-publish-";
const MARKER: &str = "install-marker";
const MAX_STAGES: usize = 128;
const MAX_ENTRIES: usize = 4096;
const MAX_MARKER_BYTES: u64 = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InstallRecoveryReport {
    pub scanned: usize,
    pub restored: usize,
    pub cleaned: usize,
    pub preserved: usize,
}

/// Write the target identity durably before publication can move the old copy.
/// The marker is metadata, not authority: recovery verifies its grammar and
/// filesystem shape before using it.
pub(super) fn write_install_marker(
    stage: &Path,
    name: &str,
    replacing: bool,
) -> std::io::Result<()> {
    if !super::is_valid_name(name) || name.len() > 128 {
        return Err(std::io::Error::other("invalid install marker name"));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join(MARKER))?;
    file.write_all(format!("1\n{name}\n{}\n", u8::from(replacing)).as_bytes())?;
    file.sync_all()?;
    sync_dir(stage)
}

/// Persist directory-entry changes where the platform supports directory
/// fsync. File content durability is the copy operation's separate concern.
pub(super) fn sync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Never traverse a repo-controlled symlink to install/recover packages.
pub(super) fn install_parent(root: &Path, create: bool) -> std::io::Result<Option<PathBuf>> {
    let mut path = match root.canonicalize() {
        Ok(path) => path,
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    for part in ["oxplow", "extensions"] {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(std::io::Error::other("extension install parent is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                std::fs::create_dir(&path)?;
                sync_dir(path.parent().expect("project directory has a parent"))?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Some(path))
}

/// Recover interrupted staged publications before the catalog is first read.
/// A missing target is restored only from an identified old copy. A candidate
/// from a first install is never made live by recovery.
pub fn recover_installations(
    root: &Path,
) -> Result<InstallRecoveryReport, oxplow_domain::DomainError> {
    let _guard = InstallLock::acquire(root).map_err(|_| recovery_error())?;
    let Some(parent) = install_parent(root, false).map_err(|_| recovery_error())? else {
        return Ok(InstallRecoveryReport::default());
    };
    let entries = match std::fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(InstallRecoveryReport::default())
        }
        Err(_) => return Err(recovery_error()),
    };
    let mut stages = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(recovery_error());
        }
        let entry = entry.map_err(|_| recovery_error())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(STAGE_PREFIX) {
            continue;
        }
        stages.push(entry.path());
        if stages.len() > MAX_STAGES {
            return Err(recovery_error());
        }
    }
    stages.sort();
    let mut report = InstallRecoveryReport::default();
    for stage in stages {
        report.scanned += 1;
        match recover_stage(&parent, &stage) {
            Ok(RecoveryChoice::Restored) => report.restored += 1,
            Ok(RecoveryChoice::Cleaned) => report.cleaned += 1,
            Ok(RecoveryChoice::Preserved) | Err(_) => {
                report.preserved += 1;
                tracing::warn!("extension install staging needs manual recovery");
            }
        }
    }
    Ok(report)
}

fn recovery_error() -> oxplow_domain::DomainError {
    oxplow_domain::DomainError::Storage("extension install recovery failed".into())
}

#[derive(Clone, Copy)]
enum RecoveryChoice {
    Restored,
    Cleaned,
    Preserved,
}

fn recover_stage(parent: &Path, stage: &Path) -> std::io::Result<RecoveryChoice> {
    if !is_plain_dir(stage)? {
        return Ok(RecoveryChoice::Preserved);
    }
    for (index, entry) in std::fs::read_dir(stage)?.enumerate() {
        let entry = entry?;
        if index >= 3
            || !matches!(
                entry.file_name().to_str(),
                Some(MARKER | "candidate" | "previous")
            )
        {
            return Ok(RecoveryChoice::Preserved);
        }
    }
    let Some((name, replacing)) = read_marker(stage)? else {
        return Ok(RecoveryChoice::Preserved);
    };
    let candidate = stage.join("candidate");
    let previous = stage.join("previous");
    let target = parent.join(&name);
    let candidate_present = plain_dir_if_present(&candidate)?;
    let previous_present = plain_dir_if_present(&previous)?;
    let target_present = plain_dir_if_present(&target)?;

    if replacing && previous_present && !target_present {
        std::fs::rename(&previous, &target)?;
        sync_dir(stage)?;
        sync_dir(parent)?;
        // The known prior copy is live again; any unpublished candidate is
        // safe to discard only after both rename barriers succeed.
        std::fs::remove_dir_all(stage)?;
        sync_dir(parent)?;
        return Ok(RecoveryChoice::Restored);
    }
    let completed = if replacing {
        // A target beside `previous` may have been created by another
        // process. Without a content proof, preserve the known old copy.
        !previous_present && target_present
    } else {
        !previous_present && (target_present != candidate_present)
    };
    if completed {
        std::fs::remove_dir_all(stage)?;
        sync_dir(parent)?;
        return Ok(RecoveryChoice::Cleaned);
    }
    Ok(RecoveryChoice::Preserved)
}

fn read_marker(stage: &Path) -> std::io::Result<Option<(String, bool)>> {
    let path = stage.join(MARKER);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_MARKER_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Ok(None);
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let parts: Vec<_> = text.split('\n').collect();
    if parts.len() != 4 || parts[0] != "1" || !parts[3].is_empty() {
        return Ok(None);
    }
    let name = parts[1];
    if !super::is_valid_name(name) || name.len() > 128 {
        return Ok(None);
    }
    let replacing = match parts[2] {
        "0" => false,
        "1" => true,
        _ => return Ok(None),
    };
    Ok(Some((name.to_owned(), replacing)))
}

fn is_plain_dir(path: &Path) -> std::io::Result<bool> {
    Ok(std::fs::symlink_metadata(path)?.file_type().is_dir())
}

fn plain_dir_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(std::io::Error::other(
            "extension staging has an unsafe entry",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) struct CandidateCatalog {
    pub extensions: Vec<Extension>,
    target: usize,
    pub dependents: BTreeSet<String>,
}

impl CandidateCatalog {
    pub fn candidate(&self) -> &Extension {
        &self.extensions[self.target]
    }

    pub fn into_candidate(mut self) -> Extension {
        self.extensions.swap_remove(self.target)
    }
}

pub(crate) fn build(root: &Path, project: &Path, mut candidate: Extension) -> CandidateCatalog {
    let mut sources = super::load_sources(root);
    let disabled = oxplow_config::disabled_extensions(project);
    let baseline = super::resolve_catalog(sources.clone(), &disabled);
    let previously_active: BTreeSet<_> = baseline
        .iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.name.clone())
        .collect();
    let name = candidate.name.clone();
    let origin = candidate.origin.clone();
    // Authoring previews inspect the candidate, including a disabled package,
    // without changing the project's enabling policy or publishing authority.
    candidate.enabled = true;
    if let Some(index) = sources
        .iter()
        .position(|ext| ext.name == name && ext.origin == origin)
    {
        sources[index] = candidate;
    } else {
        sources.push(candidate);
    }
    let preview_disabled: Vec<_> = disabled
        .into_iter()
        .filter(|disabled| disabled != &name)
        .collect();
    let mut extensions = super::resolve_catalog(sources, &preview_disabled);
    let target = extensions
        .iter()
        .position(|ext| ext.name == name && ext.origin == origin)
        .expect("candidate remains listed even when blocked");
    let mut regressions = Vec::new();
    for ext in extensions.iter().filter(|ext| {
        ext.name != name && previously_active.contains(&ext.name) && ext.public_contracts.blocked
    }) {
        regressions.extend(
            ext.errors
                .iter()
                .map(|error| format!("candidate `{name}` blocks consumer `{}`: {error}", ext.name)),
        );
    }
    for error in regressions {
        if !extensions[target].errors.contains(&error) {
            extensions[target].errors.push(error);
        }
    }
    let mut dependents = BTreeSet::from([name]);
    loop {
        let consumers: Vec<_> = extensions
            .iter()
            .filter(|ext| {
                ext.public_contracts
                    .imports
                    .iter()
                    .any(|import| dependents.contains(&import.contribution.extension))
            })
            .map(|ext| ext.name.clone())
            .collect();
        let before = dependents.len();
        dependents.extend(consumers);
        if before == dependents.len() {
            break;
        }
    }
    CandidateCatalog {
        extensions,
        target,
        dependents,
    }
}
