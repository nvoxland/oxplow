//! Bounded private code copies for explicitly approved broader programs.
//! Working-tree edits cannot substitute the entry or its relative helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::exec_consent::{ApprovalStore, Gate, ProgramKind, ProjectProgram};
use crate::extensions::{Disk, ExtensionFiles};

/// One project's collector program; the gate owns the lifetime of its copy.
pub(crate) struct ProjectCode {
    pub entry: PathBuf,
    pub location: oxplow_script::runtime::ExecLocation,
    pub gate: Gate,
}

/// Prepare only the code the approval covers. A root-level entry covers its
/// own file; an entry in a directory also covers that directory's helpers.
pub(crate) fn project_collector(
    approvals: Arc<ApprovalStore>,
    root: PathBuf,
    program: ProjectProgram,
) -> Result<ProjectCode, String> {
    program.execution.require_external()?;
    if program.kind != ProgramKind::Collector
        || !program.args.is_empty()
        || program.remote
        || !oxplow_domain::service::valid_path(&program.program)
    {
        return Err("unsupported project collector code location".into());
    }
    let hash = program.hash(&root).map_err(|e| e.to_string())?;
    let consent = approvals.pinned_gate(program.key(), hash.clone());
    consent()?;
    let copy = Arc::new(
        tempfile::Builder::new()
            .prefix("oxplow-project-code-")
            .tempdir()
            .map_err(|e| e.to_string())?,
    );
    if let Some(parent) = Path::new(&program.program)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        copy_directory(&root.join(parent), &copy.path().join(parent)).map_err(|e| e.to_string())?;
    } else {
        let mut total = 0;
        copy_file(
            &Disk(root.clone()),
            &program.program,
            copy.path(),
            &mut total,
        )
        .map_err(|e| e.to_string())?;
    }
    if program.hash(copy.path()).map_err(|e| e.to_string())? != hash {
        return Err(
            "the program changed while preparing its approved copy; review it again".into(),
        );
    }
    let entry = copy.path().join(&program.program);
    let location = oxplow_script::runtime::ExecLocation {
        directory: copy.path().to_path_buf(),
        project: root.clone(),
    };
    let gate: Gate = Arc::new(move || {
        consent()?;
        if program.hash(&root).is_ok_and(|current| current == hash)
            && program
                .hash(copy.path())
                .is_ok_and(|current| current == hash)
        {
            Ok(())
        } else {
            Err("the approved program or its private copy changed during the run".into())
        }
    });
    gate()?;
    Ok(ProjectCode {
        entry,
        location,
        gate,
    })
}

/// Copy regular files only, retaining normal permission bits and the same
/// 500-file/16-MiB limits as approval hashing. The destination is host-private.
pub(crate) fn copy_directory(directory: &Path, destination: &Path) -> std::io::Result<()> {
    let source = Disk(directory.to_path_buf());
    let paths = source.paths()?;
    if paths.len() > crate::exec_consent::TREE_MAX_FILES {
        return Err(std::io::Error::other(
            "too many files to copy the code safely",
        ));
    }
    std::fs::create_dir_all(destination)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o700))?;
    }
    // Relative helpers can depend on empty directories as well as files.
    for entry in walkdir::WalkDir::new(directory).follow_links(false) {
        let entry = entry.map_err(std::io::Error::other)?;
        if entry.file_type().is_symlink() {
            return Err(std::io::Error::other("code copy contains a symlink"));
        }
        if entry.file_type().is_dir() {
            let rel = entry
                .path()
                .strip_prefix(directory)
                .map_err(std::io::Error::other)?;
            std::fs::create_dir_all(destination.join(rel))?;
        }
    }
    let mut total = 0;
    for rel in paths {
        copy_file(&source, &rel, destination, &mut total)?;
    }
    Ok(())
}

fn copy_file(source: &Disk, rel: &str, destination: &Path, total: &mut u64) -> std::io::Result<()> {
    if !oxplow_domain::service::valid_path(rel) {
        return Err(std::io::Error::other(
            "code copy contains a non-relative path",
        ));
    }
    let bytes = source.bytes(rel)?;
    *total += bytes.len() as u64;
    if *total > crate::exec_consent::TREE_MAX_BYTES {
        return Err(std::io::Error::other("too large to copy the code safely"));
    }
    let metadata = std::fs::symlink_metadata(source.0.join(rel))?;
    if !metadata.is_file() {
        return Err(std::io::Error::other(
            "code copy contains a non-regular file",
        ));
    }
    let target = destination.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, bytes)?;
    let permissions = metadata.permissions();
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = permissions;
        permissions.set_mode(permissions.mode() & 0o777);
        permissions
    };
    std::fs::set_permissions(target, permissions)
}
