//! The deployment store (`.context/extensions.md` "Deployments"): each
//! deployment of an extension is an immutable copy of its source at
//! `<project>/.oxplow/runtime/extensions/<name>/<hash>/`, where `<hash>` is
//! the hash of the source it was copied from. Everything oxplow runs or
//! serves for a deployed extension is read from that folder. The file
//! watcher never looks under `.oxplow/runtime/`, and nothing writes into a
//! deployment after it is made.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The store, under a project.
pub const STORE_DIR: &str = ".oxplow/runtime/extensions";

/// How many deployments of an extension the store keeps beside the active one.
pub const KEEP: usize = 5;

/// The folder of `name`'s deployment `hash`.
pub fn deployment_dir(project: &Path, name: &str, hash: &str) -> PathBuf {
    project.join(STORE_DIR).join(name).join(hash)
}

/// The hash of a source folder: sha256 over each file's path (relative,
/// `/`-separated) and bytes, in path order, skipping every entry whose name
/// starts with `.`. Cut to 16 hex characters: it names a folder.
pub fn source_hash(source: &Path) -> std::io::Result<String> {
    let files = source_files(source)?
        .into_iter()
        .map(|(rel, path)| Ok((rel, std::fs::read(&path)?)))
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(files_hash(
        files
            .iter()
            .map(|(rel, bytes)| (rel.as_str(), bytes.as_slice())),
    ))
}

/// How many bytes a deployment of `source` copies.
pub fn source_size(source: &Path) -> std::io::Result<u64> {
    source_files(source)?
        .into_iter()
        .map(|(_, path)| Ok(std::fs::metadata(path)?.len()))
        .sum()
}

/// The hash of `(relative path, bytes)` files, by [`source_hash`]'s rule.
pub fn files_hash<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut files: Vec<(&str, &[u8])> = files
        .into_iter()
        .filter(|(rel, _)| !rel.split('/').any(|part| part.starts_with('.')))
        .collect();
    files.sort();
    let mut hasher = Sha256::new();
    for (rel, bytes) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hex::encode(hasher.finalize())[..16].to_string()
}

/// Write a bundled extension into the store as its shipped deployment,
/// unless that hash is there already, and return the hash. The same source
/// in the repo's `extensions/<name>/` has the same hash.
pub fn extract_shipped(
    project: &Path,
    bundled: &crate::bundled_extensions::BundledExtension,
) -> std::io::Result<String> {
    let hash = files_hash(bundled.files.iter().copied());
    let target = deployment_dir(project, bundled.name, &hash);
    if target.is_dir() {
        return Ok(hash);
    }
    write_whole(&target, |staging| {
        for (rel, bytes) in bundled.files {
            let to = staging.join(rel);
            std::fs::create_dir_all(to.parent().expect("a file has a parent"))?;
            std::fs::write(&to, bytes)?;
        }
        Ok(())
    })?;
    Ok(hash)
}

/// Fill `target` through `fill` on a folder beside it, then rename it into
/// place, so `target` is either whole or absent.
fn write_whole(
    target: &Path,
    fill: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = target.parent().expect("a deployment has a parent");
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let written = std::fs::create_dir_all(&staging)
        .and_then(|()| fill(&staging))
        .and_then(|()| std::fs::rename(&staging, target));
    if let Err(error) = written {
        let _ = std::fs::remove_dir_all(&staging);
        // Another writer of the same hash may have won the rename.
        if target.is_dir() {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

/// Every file under `source` as `(relative path, absolute path)`, in path
/// order, without dot entries. Symlinks are refused: a deployment is a copy
/// of files, not of what they point at.
fn source_files(source: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(std::io::Error::other(format!(
                    "`{rel}` is a symlink; an extension's source holds only files and folders"
                )));
            }
            if kind.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else {
                out.push((rel, entry.path()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(source, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// Copy `source` into the store as `name`'s deployment `hash`, unless it is
/// there already. The copy is written beside it and renamed into place, so
/// a deployment folder is either whole or absent.
pub fn copy_into_store(
    project: &Path,
    name: &str,
    source: &Path,
    hash: &str,
) -> std::io::Result<PathBuf> {
    let target = deployment_dir(project, name, hash);
    if target.is_dir() {
        return Ok(target);
    }
    write_whole(&target, |staging| {
        for (rel, path) in source_files(source)? {
            let to = staging.join(&rel);
            std::fs::create_dir_all(to.parent().expect("a file has a parent"))?;
            std::fs::copy(&path, &to)?;
        }
        Ok(())
    })?;
    Ok(target)
}

/// Remove `name`'s deployment folders `hashes`.
pub fn remove(project: &Path, name: &str, hashes: &[String]) {
    for hash in hashes {
        let _ = std::fs::remove_dir_all(deployment_dir(project, name, hash));
    }
}

/// Remove every deployment of `name`.
pub fn remove_all(project: &Path, name: &str) {
    let _ = std::fs::remove_dir_all(project.join(STORE_DIR).join(name));
}

/// Of `name`'s deployments, newest first, the hashes past the active one and
/// the [`KEEP`] newest.
pub fn beyond_kept(deployments: &[(String, bool)]) -> Vec<String> {
    let mut kept = 0;
    deployments
        .iter()
        .filter(|(_, active)| {
            if *active {
                return false;
            }
            kept += 1;
            kept > KEEP
        })
        .map(|(hash, _)| hash.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_same_source_has_the_same_hash_and_any_change_changes_it() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for dir in [a.path(), b.path()] {
            write(dir, "extension.yaml", "name: x\n");
            write(dir, "lenses/one.yaml", "title: One\n");
        }
        // Dot entries are not part of the source.
        write(b.path(), ".DS_Store", "noise");
        write(b.path(), ".git/HEAD", "ref");
        let hash = source_hash(a.path()).unwrap();
        assert_eq!(hash.len(), 16);
        assert_eq!(hash, source_hash(b.path()).unwrap());

        write(b.path(), "lenses/one.yaml", "title: Uno\n");
        assert_ne!(hash, source_hash(b.path()).unwrap());
        // A file moved to another folder is another source.
        let c = tempfile::tempdir().unwrap();
        write(c.path(), "extension.yaml", "name: x\n");
        write(c.path(), "one.yaml", "title: One\n");
        assert_ne!(hash, source_hash(c.path()).unwrap());
    }

    #[test]
    fn a_copy_lands_whole_once_and_never_changes() {
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "extension.yaml", "name: x\n");
        write(source.path(), "lenses/one.yaml", "title: One\n");
        write(source.path(), ".hidden", "skip");
        let hash = source_hash(source.path()).unwrap();

        let dir = copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert_eq!(dir, deployment_dir(project.path(), "x", &hash));
        assert_eq!(
            std::fs::read_to_string(dir.join("lenses/one.yaml")).unwrap(),
            "title: One\n"
        );
        assert!(!dir.join(".hidden").exists());

        // The same hash again leaves the deployment as it was.
        write(source.path(), "lenses/one.yaml", "title: changed\n");
        copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("lenses/one.yaml")).unwrap(),
            "title: One\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, [hash]);
    }

    #[cfg(unix)]
    #[test]
    fn a_source_with_a_symlink_is_refused() {
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "extension.yaml", "name: x\n");
        std::os::unix::fs::symlink("/etc/hosts", source.path().join("hosts")).unwrap();
        let err = source_hash(source.path()).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
    }

    #[test]
    fn a_shipped_extension_lands_on_disk_with_its_source_folders_hash() {
        let project = tempfile::tempdir().unwrap();
        for bundled in crate::bundled_extensions::BUNDLED {
            let hash = extract_shipped(project.path(), bundled).unwrap();
            let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../extensions")
                .join(bundled.name);
            assert_eq!(hash, source_hash(&repo).unwrap(), "{}", bundled.name);
            let dir = deployment_dir(project.path(), bundled.name, &hash);
            assert_eq!(
                std::fs::read(dir.join("extension.yaml")).unwrap(),
                bundled.bytes("extension.yaml").unwrap()
            );
            assert_eq!(extract_shipped(project.path(), bundled).unwrap(), hash);
        }
    }

    #[test]
    fn the_store_keeps_the_active_deployment_and_the_five_newest() {
        let deployments: Vec<(String, bool)> = (1..=8).map(|i| (format!("h{i}"), i == 7)).collect();
        assert_eq!(beyond_kept(&deployments), ["h6", "h8"]);
        let more: Vec<(String, bool)> = (1..=9).map(|i| (format!("h{i}"), i == 1)).collect();
        assert_eq!(beyond_kept(&more), ["h7", "h8", "h9"]);
    }
}
