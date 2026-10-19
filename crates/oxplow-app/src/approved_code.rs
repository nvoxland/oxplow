//! Bounded private code copies for approved external programs.
//! Working-tree edits cannot substitute the entry or its relative helpers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::exec_consent::{ApprovalStore, Gate};
use crate::extensions::{Disk, ExtensionFiles};

/// An approved collector program's private copy; the gate owns its lifetime.
pub(crate) struct ApprovedCode {
    pub entry: PathBuf,
    pub location: oxplow_script::runtime::ExecLocation,
    pub gate: Gate,
}

/// The project's own extension's collector `spec`, in `ext_dir` (its
/// extension, `owner`), as an approved copy to run: refused until a person
/// approved it at its current version (`collector_runner::approval_hash`
/// — every file the extension runs and what its declaration grants). The
/// gate holds while the approval stands and neither the extension nor the
/// copy changed.
pub(crate) fn extension_collector(
    approvals: Arc<ApprovalStore>,
    root: PathBuf,
    ext_dir: PathBuf,
    owner: &str,
    spec: &oxplow_config::collectors::CollectorSpec,
) -> Result<ApprovedCode, String> {
    use crate::collector_runner::{approval_hash, is_approved};
    let entry = spec.entry.clone().unwrap_or_default();
    if !oxplow_domain::service::valid_path(&entry) {
        return Err("unsupported collector code location".into());
    }
    let hash = approval_hash(&ext_dir, spec).map_err(|e| e.to_string())?;
    let key = (owner.to_string(), spec.id.clone());
    let approved = {
        let approvals = approvals.clone();
        let (key, hash) = (key.clone(), hash.clone());
        move || {
            if is_approved(&approvals, &key.0, &key.1, &hash) {
                Ok(())
            } else {
                Err(format!(
                    "collector `{}/{}` runs a program and needs a person's approval at this \
                     version first (Settings → Data); it was never approved, changed since, or \
                     was revoked",
                    key.0, key.1
                ))
            }
        }
    };
    approved()?;
    let copy = Arc::new(
        tempfile::Builder::new()
            .prefix("oxplow-collector-")
            .tempdir()
            .map_err(|e| e.to_string())?,
    );
    copy_directory(&ext_dir, copy.path()).map_err(|e| e.to_string())?;
    if approval_hash(copy.path(), spec).map_err(|e| e.to_string())? != hash {
        return Err(
            "the program changed while preparing its approved copy; review it again".into(),
        );
    }
    let location = oxplow_script::runtime::ExecLocation {
        directory: copy.path().to_path_buf(),
        project: root,
    };
    let spec = spec.clone();
    let gate: Gate = Arc::new(move || {
        approved()?;
        if approval_hash(&ext_dir, &spec).is_ok_and(|current| current == hash)
            && approval_hash(copy.path(), &spec).is_ok_and(|current| current == hash)
        {
            Ok(())
        } else {
            Err("the approved program or its private copy changed during the run".into())
        }
    });
    gate()?;
    Ok(ApprovedCode {
        entry: location.directory.join(&entry),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_consent::ApprovalStore;

    fn spec(entry: &str) -> oxplow_config::collectors::CollectorSpec {
        let yaml = format!(
            "- {{ id: repo.count, runtime: exec, entry: \"{entry}\", trigger: manual, facts: [repo.count] }}"
        );
        let (mut specs, errors) = oxplow_config::collectors::parse_collectors(
            &oxplow_config::collectors::Declarer::own("project"),
            &serde_yaml::from_str(&yaml).unwrap(),
            &|_| true,
        );
        assert_eq!(errors, Vec::<String>::new());
        specs.remove(0)
    }

    /// A project whose own extension `project` has a program and a helper.
    fn project() -> (tempfile::TempDir, PathBuf, Arc<ApprovalStore>) {
        let root = tempfile::tempdir().unwrap();
        let ext = root.path().join(".oxplow/extensions/project");
        std::fs::create_dir_all(ext.join("tools")).unwrap();
        std::fs::write(ext.join("tools/count.sh"), "echo one").unwrap();
        std::fs::write(ext.join("tools/helper.txt"), "helper").unwrap();
        let approvals = Arc::new(ApprovalStore::for_tests(root.path()));
        (root, ext, approvals)
    }

    fn approve(
        approvals: &ApprovalStore,
        ext: &Path,
        spec: &oxplow_config::collectors::CollectorSpec,
    ) {
        let hash = crate::collector_runner::approval_hash(ext, spec).unwrap();
        crate::collector_runner::approve(approvals, "project", &spec.id, &hash).unwrap();
    }

    fn code(
        root: &Path,
        ext: &Path,
        approvals: &Arc<ApprovalStore>,
        spec: &oxplow_config::collectors::CollectorSpec,
    ) -> Result<ApprovedCode, String> {
        extension_collector(
            approvals.clone(),
            root.to_path_buf(),
            ext.to_path_buf(),
            "project",
            spec,
        )
    }

    fn refused(result: Result<ApprovedCode, String>) -> String {
        match result {
            Err(message) => message,
            Ok(_) => panic!("expected a refusal"),
        }
    }

    #[test]
    fn nothing_is_copied_until_a_person_approved_it() {
        let (root, ext, approvals) = project();
        let message = refused(code(root.path(), &ext, &approvals, &spec("tools/count.sh")));
        assert!(message.contains("approval"), "{message}");
    }

    #[test]
    fn an_approved_program_runs_from_a_copy_until_it_is_edited_or_revoked() {
        let (root, ext, approvals) = project();
        let spec = spec("tools/count.sh");
        approve(&approvals, &ext, &spec);
        let code1 = code(root.path(), &ext, &approvals, &spec).unwrap();

        // It runs beside a private copy holding the entry and its helpers.
        assert!(!code1.entry.starts_with(root.path()));
        assert_eq!(std::fs::read_to_string(&code1.entry).unwrap(), "echo one");
        assert_eq!(
            std::fs::read_to_string(code1.location.directory.join("tools/helper.txt")).unwrap(),
            "helper"
        );
        assert_eq!(code1.location.project, root.path());
        (code1.gate)().expect("approved and unchanged");

        // Editing the extension under a running program ends its authority;
        // the copy itself never changes.
        std::fs::write(ext.join("tools/helper.txt"), "edited").unwrap();
        assert!((code1.gate)().is_err());
        assert_eq!(
            std::fs::read_to_string(code1.location.directory.join("tools/helper.txt")).unwrap(),
            "helper"
        );
        // The edited program is a different version: unapproved until reviewed.
        let message = refused(code(root.path(), &ext, &approvals, &spec));
        assert!(message.contains("approval"), "{message}");

        // Revocation stops a program approved at its current version.
        approve(&approvals, &ext, &spec);
        let code2 = code(root.path(), &ext, &approvals, &spec).unwrap();
        (code2.gate)().unwrap();
        approvals.revoke("project/repo.count").unwrap();
        assert!((code2.gate)().is_err());

        // The copy goes with the last holder of the gate.
        let directory = code2.location.directory.clone();
        drop(code2);
        assert!(!directory.exists());
    }

    #[test]
    fn only_a_plain_extension_relative_entry_is_copied() {
        let (root, ext, approvals) = project();
        let mut spec = spec("tools/count.sh");
        for bad in ["../outside.sh", "/abs/path.sh", "tools/../../x.sh"] {
            spec.entry = Some(bad.into());
            let message = refused(code(root.path(), &ext, &approvals, &spec));
            assert!(message.contains("location"), "{bad}: {message}");
        }
    }
}
