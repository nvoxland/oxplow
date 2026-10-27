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

/// What of a source is its input (`.context/extensions.md` "Builds"):
/// with a `build:`, the output it writes and the folders it excludes
/// aren't, and its command is. Read from `extension.yaml` leniently: a
/// manifest that doesn't parse has no build, and its load says why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inputs {
    pub run: Option<String>,
    pub output: Option<String>,
    pub exclude: Vec<String>,
}

impl Inputs {
    pub fn of(source: &Path) -> Inputs {
        std::fs::read_to_string(source.join("extension.yaml"))
            .map(|text| Self::of_text(&text))
            .unwrap_or_default()
    }

    /// The inputs a manifest's text declares.
    pub fn of_text(manifest: &str) -> Inputs {
        let build = serde_yaml::from_str::<serde_yaml::Value>(manifest)
            .ok()
            .and_then(|manifest| manifest.get("build").cloned());
        let Some(build) = build else {
            return Inputs::default();
        };
        let text = |key: &str| build.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Inputs {
            run: text("run"),
            output: text("output"),
            exclude: build
                .get("exclude")
                .and_then(|v| v.as_sequence())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The folders that aren't inputs: the output and the excluded ones.
    fn skipped(&self) -> Vec<&str> {
        self.output
            .iter()
            .chain(&self.exclude)
            .map(|dir| dir.trim_end_matches('/'))
            .collect()
    }
}

/// The hash of a source folder: sha256 over each input file's path
/// (relative, `/`-separated) and bytes, in path order, skipping every entry
/// whose name starts with `.`, plus a built source's command ([`Inputs`]).
/// Cut to 16 hex characters: it names a folder.
pub fn source_hash(source: &Path) -> std::io::Result<String> {
    let inputs = Inputs::of(source);
    let mut files = source_files(source, &inputs.skipped())?
        .into_iter()
        .map(|(rel, path)| Ok((rel, std::fs::read(&path)?)))
        .collect::<std::io::Result<Vec<_>>>()?;
    if let Some(run) = &inputs.run {
        // Not a path any file can have.
        files.push(("\0build".to_string(), run.clone().into_bytes()));
    }
    Ok(files_hash(
        files
            .iter()
            .map(|(rel, bytes)| (rel.as_str(), bytes.as_slice())),
    ))
}

/// How many bytes of input a deployment of `source` copies.
pub fn source_size(source: &Path) -> std::io::Result<u64> {
    source_files(source, &Inputs::of(source).skipped())?
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

/// The hash of the copy of `bundled` built into oxplow, by
/// [`source_hash`]'s rule: its inputs and its build command, never what
/// the build wrote — so a deploy of the same source folder resolves to
/// it.
pub fn shipped_hash(bundled: &crate::bundled_extensions::BundledExtension) -> String {
    let inputs = bundled
        .file("extension.yaml")
        .map(Inputs::of_text)
        .unwrap_or_default();
    let skipped = inputs.skipped();
    let mut files: Vec<(&str, &[u8])> = bundled
        .files()
        .iter()
        .copied()
        .filter(|(rel, _)| {
            !skipped
                .iter()
                .any(|dir| *rel == *dir || rel.starts_with(&format!("{dir}/")))
        })
        .collect();
    if let Some(run) = &inputs.run {
        files.push(("\0build", run.as_bytes()));
    }
    files_hash(files)
}

/// Write a bundled extension into the store as its shipped deployment,
/// unless that hash is there already, and return the hash. The same source
/// in the repo's `extensions/<name>/` has the same hash.
pub fn extract_shipped(
    project: &Path,
    bundled: &crate::bundled_extensions::BundledExtension,
) -> std::io::Result<String> {
    let hash = shipped_hash(bundled);
    let target = deployment_dir(project, bundled.name, &hash);
    if target.is_dir() {
        return Ok(hash);
    }
    write_whole(&target, |staging| {
        for (rel, bytes) in bundled.files() {
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
        .and_then(|()| set_read_only(&staging, true))
        .and_then(|()| std::fs::rename(&staging, target));
    if let Err(error) = written {
        remove_tree(&staging);
        // Another writer of the same hash may have won the rename.
        if target.is_dir() {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

/// Make every file under `dir` read-only (the write bits off, the rest
/// kept), or writable again. A deployment's files never change: a program
/// running from it can't rewrite what it runs by accident. Its folders stay
/// writable — a same-user process could undo either with `chmod`, so this
/// guards against mistakes, not against a program — and so a temporary
/// project holding one still cleans up.
fn set_read_only(dir: &Path, read_only: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                set_read_only(&path, read_only)?;
            } else {
                let mode = meta.permissions().mode();
                let mode = if read_only {
                    mode & !0o222
                } else {
                    mode | 0o200
                };
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, read_only);
    }
    Ok(())
}

/// Remove `dir`, a deployment or the folder of them.
fn remove_tree(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// The files a bundled extension ships from its folder `source`, as
/// `(relative path, bytes)` in path order: every file but dot entries and
/// the manifest's `build.exclude` — its build's output is shipped. Only a
/// debug build reads them at run time (`bundled_extensions`).
#[cfg(debug_assertions)]
pub(crate) fn shipped_files(source: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let exclude: Vec<String> = Inputs::of(source)
        .exclude
        .iter()
        .map(|dir| dir.trim_end_matches('/').to_string())
        .collect();
    let exclude: Vec<&str> = exclude.iter().map(String::as_str).collect();
    source_files(source, &exclude)?
        .into_iter()
        .map(|(rel, path)| Ok((rel, std::fs::read(path)?)))
        .collect()
}

/// Every file under `source` as `(relative path, absolute path)`, in path
/// order, without dot entries or the `skip` folders (relative paths).
/// Symlinks are refused: a deployment is a copy of files, not of what they
/// point at.
fn source_files(source: &Path, skip: &[&str]) -> std::io::Result<Vec<(String, PathBuf)>> {
    fn walk(
        dir: &Path,
        prefix: &str,
        skip: &[&str],
        out: &mut Vec<(String, PathBuf)>,
    ) -> std::io::Result<()> {
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
            if skip.contains(&rel.as_str()) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(std::io::Error::other(format!(
                    "`{rel}` is a symlink; an extension's source holds only files and folders"
                )));
            }
            if kind.is_dir() {
                walk(&entry.path(), &rel, skip, out)?;
            } else {
                out.push((rel, entry.path()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(source, "", skip, &mut out)?;
    out.sort();
    Ok(out)
}

/// Copy `source` into the store as `name`'s deployment `hash`, unless it is
/// there already: its inputs, and a built source's output. The copy is
/// written beside it and renamed into place, so a deployment folder is
/// either whole or absent.
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
    let inputs = Inputs::of(source);
    write_whole(&target, |staging| {
        let mut files = source_files(source, &inputs.skipped())?;
        if let Some(output) = inputs.output.as_deref().map(|o| o.trim_end_matches('/')) {
            if source.join(output).is_dir() {
                files.extend(
                    source_files(&source.join(output), &[])?
                        .into_iter()
                        .map(|(rel, path)| (format!("{output}/{rel}"), path)),
                );
            }
        }
        for (rel, path) in files {
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
        remove_tree(&deployment_dir(project, name, hash));
    }
}

/// Remove every deployment of `name`.
pub fn remove_all(project: &Path, name: &str) {
    remove_tree(&project.join(STORE_DIR).join(name));
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
            write(dir, "pages/one.yaml", "title: One\n");
        }
        // Dot entries are not part of the source.
        write(b.path(), ".DS_Store", "noise");
        write(b.path(), ".git/HEAD", "ref");
        let hash = source_hash(a.path()).unwrap();
        assert_eq!(hash.len(), 16);
        assert_eq!(hash, source_hash(b.path()).unwrap());

        write(b.path(), "pages/one.yaml", "title: Uno\n");
        assert_ne!(hash, source_hash(b.path()).unwrap());
        // A file moved to another folder is another source.
        let c = tempfile::tempdir().unwrap();
        write(c.path(), "extension.yaml", "name: x\n");
        write(c.path(), "one.yaml", "title: One\n");
        assert_ne!(hash, source_hash(c.path()).unwrap());
    }

    /// A manifest with a `build` hashes its inputs and its command: not
    /// what the build wrote, nor the folders it leaves out. One without a
    /// `build` hashes every file, as it always has.
    #[test]
    fn the_build_command_and_only_the_inputs_make_the_hash() {
        let plain = tempfile::tempdir().unwrap();
        write(plain.path(), "extension.yaml", "name: x\n");
        write(plain.path(), "dist/a.js", "built");
        assert_eq!(
            source_hash(plain.path()).unwrap(),
            files_hash([
                ("extension.yaml", b"name: x\n".as_slice()),
                ("dist/a.js", b"built".as_slice()),
            ])
        );

        let built = tempfile::tempdir().unwrap();
        let manifest = |run: &str| {
            format!("name: x\nbuild: {{ run: {run}, output: dist, exclude: [node_modules] }}\n")
        };
        write(built.path(), "extension.yaml", &manifest("make"));
        write(built.path(), "src/a.ts", "source");
        let hash = source_hash(built.path()).unwrap();
        write(built.path(), "dist/a.js", "built");
        write(built.path(), "node_modules/x/index.js", "dep");
        assert_eq!(hash, source_hash(built.path()).unwrap());
        assert_eq!(
            source_size(built.path()).unwrap(),
            (manifest("make").len() + "source".len()) as u64
        );
        write(built.path(), "src/a.ts", "changed");
        assert_ne!(hash, source_hash(built.path()).unwrap());
        write(built.path(), "src/a.ts", "source");
        write(built.path(), "extension.yaml", &manifest("other"));
        let other = source_hash(built.path()).unwrap();
        write(built.path(), "extension.yaml", &manifest("make"));
        assert_eq!(hash, source_hash(built.path()).unwrap());
        assert_ne!(
            other,
            files_hash([
                ("extension.yaml", manifest("other").as_bytes()),
                ("src/a.ts", b"source".as_slice()),
            ])
        );
    }

    /// The deployment of a built source holds its inputs and the build's
    /// output, never the folders it excludes.
    #[test]
    fn a_copy_carries_the_built_output_and_not_the_excluded_folders() {
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        write(
            source.path(),
            "extension.yaml",
            "name: x\nbuild: { run: make, output: dist, exclude: [node_modules] }\n",
        );
        write(source.path(), "src/a.ts", "source");
        write(source.path(), "dist/w/index.html", "built");
        write(source.path(), "node_modules/x/index.js", "dep");
        let hash = source_hash(source.path()).unwrap();
        let dir = copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("dist/w/index.html")).unwrap(),
            "built"
        );
        assert!(dir.join("src/a.ts").is_file());
        assert!(!dir.join("node_modules").exists());
    }

    /// The copy built into oxplow hashes as its source folder does: by its
    /// inputs and build command, not what the build wrote.
    #[test]
    fn a_shipped_copy_with_a_build_hashes_by_its_inputs() {
        const MANIFEST: &str =
            "name: x\nbuild: { run: make, output: dist, exclude: [node_modules] }\n";
        const FILES: crate::bundled_extensions::Files = &[
            ("dist/w/index.html", b"built"),
            ("extension.yaml", MANIFEST.as_bytes()),
            ("src/a.ts", b"source"),
        ];
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "extension.yaml", MANIFEST);
        write(source.path(), "src/a.ts", "source");
        write(source.path(), "dist/w/index.html", "built");
        let bundled = crate::bundled_extensions::BundledExtension {
            name: "x",
            files: || FILES,
            required: false,
        };
        assert_eq!(shipped_hash(&bundled), source_hash(source.path()).unwrap());
        // And it ships what the build wrote.
        let project = tempfile::tempdir().unwrap();
        let hash = extract_shipped(project.path(), &bundled).unwrap();
        assert!(deployment_dir(project.path(), "x", &hash)
            .join("dist/w/index.html")
            .is_file());
    }

    /// A deployment's files are read-only once written: a program running
    /// from it can't rewrite what it runs by accident. Pruning still
    /// removes it.
    #[cfg(unix)]
    #[test]
    fn a_deployment_is_read_only_and_still_pruned() {
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "extension.yaml", "name: x\n");
        write(source.path(), "bin/run.sh", "#!/bin/sh\n");
        let hash = source_hash(source.path()).unwrap();
        let dir = copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert!(std::fs::write(dir.join("bin/run.sh"), "changed").is_err());
        assert!(std::fs::write(dir.join("extension.yaml"), "changed").is_err());
        remove(project.path(), "x", &[hash]);
        assert!(!dir.exists());
    }

    #[test]
    fn a_copy_lands_whole_once_and_never_changes() {
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "extension.yaml", "name: x\n");
        write(source.path(), "pages/one.yaml", "title: One\n");
        write(source.path(), ".hidden", "skip");
        let hash = source_hash(source.path()).unwrap();

        let dir = copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert_eq!(dir, deployment_dir(project.path(), "x", &hash));
        assert_eq!(
            std::fs::read_to_string(dir.join("pages/one.yaml")).unwrap(),
            "title: One\n"
        );
        assert!(!dir.join(".hidden").exists());

        // The same hash again leaves the deployment as it was.
        write(source.path(), "pages/one.yaml", "title: changed\n");
        copy_into_store(project.path(), "x", source.path(), &hash).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("pages/one.yaml")).unwrap(),
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
