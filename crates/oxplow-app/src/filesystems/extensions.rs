//! `oxplow:extensions`: the copies of extensions oxplow runs
//! (`.context/filesystems.md`). A path is `<extension name>/<path inside
//! it>`; the revision is a deployment's hash, and none means the
//! extension's running deployment. Read-only.
//!
//! The files are the deployment folder in the store
//! (`extension_store::deployment_dir`), immutable once made. What runs is
//! the catalog's: [`running_hash`], the same lookup that fills a page's
//! `PageProvenance.deployment`. An extension that runs from its source
//! folder (an authoring check, the CLI, a dev build's own extensions) or
//! has no deployment runs no copy, so with no revision it has no files
//! here and isn't listed; name one of its deployments by hash to read it.
//!
//! Every path is checked twice: lexically (no `..`, no absolute path) and
//! by canonicalizing the target and requiring it to lie inside the
//! canonical deployment folder, so a symlink out of a deployment reads as
//! a refusal, never as the file it points at.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::filesystem::{
    ByteRange, FileSystem, FsCx, FsEntry, FsError, FsKind, FsStat, EXTENSIONS,
};

use super::local::{read_range_on_disk, stat_on_disk};
use crate::extension::extension_catalog::ExtensionCatalog;
use crate::extension::extension_store::{deployment_dir, STORE_DIR};
use crate::extension::extensions::{owner, Extension};

/// The hash of the deployment `name` runs in `extensions`, a load's list:
/// none when it isn't there or runs no deployment.
pub fn running_hash<'a>(extensions: &'a [Extension], name: &str) -> Option<&'a str> {
    owner(extensions, name).and_then(|ext| ext.deployment.hash())
}

/// The file URI of page file `file` (`<slug>.yaml`) of extension `name`
/// as deployment `hash` has it: how `PageProvenance.running_uri` and
/// `v_page.running_uri` name the copy that runs.
pub fn page_running_uri(name: &str, file: &str, hash: &str) -> String {
    oxplow_domain::uri::build::file_uri_at(
        &format!("{name}/pages/{file}"),
        Some(EXTENSIONS),
        Some(hash),
        None,
    )
}

pub struct ExtensionsFs {
    project: PathBuf,
    catalog: Arc<ExtensionCatalog>,
}

/// Where a path lands: an extension's deployment and a path inside it.
struct Target {
    name: String,
    hash: String,
    /// Relative to the deployment folder; empty is the folder itself.
    inside: String,
}

impl ExtensionsFs {
    pub fn new(project: PathBuf, catalog: Arc<ExtensionCatalog>) -> Self {
        Self { project, catalog }
    }

    /// The hash an extension runs.
    fn running(&self, name: &str) -> Option<String> {
        let extensions = self.catalog.unpinned(
            &self.project,
            "a file's copy is read by the hash the catalog names",
        );
        running_hash(&extensions, name).map(str::to_string)
    }

    /// Where `path` is at `rev`; `None` when the extension has no such
    /// deployment (or runs none). `""` is the root and has no target.
    fn target(&self, rev: Option<&str>, path: &str) -> Result<Option<Target>, FsError> {
        let rev = valid_rev(rev)?;
        let (name, inside) = split(path)?;
        if name.is_empty() {
            return Ok(None);
        }
        let Some(hash) = (match rev {
            Some(rev) => Some(rev.to_string()),
            None => self.running(&name),
        }) else {
            return Ok(None);
        };
        Ok(Some(Target { name, hash, inside }))
    }

    fn dir_of(&self, target: &Target) -> PathBuf {
        deployment_dir(&self.project, &target.name, &target.hash)
    }

    /// The extensions that have a deployment at `rev`, by name: those that
    /// run one (none), or whose store folder holds `rev`.
    fn extension_names(&self, rev: Option<&str>) -> Vec<String> {
        match rev {
            None => {
                let extensions = self
                    .catalog
                    .unpinned(&self.project, "the extensions that run a copy");
                let mut names: Vec<String> = extensions
                    .iter()
                    .filter(|ext| ext.deployment.hash().is_some())
                    .map(|ext| ext.name.clone())
                    .collect();
                names.sort();
                names.dedup();
                names
            }
            Some(rev) => {
                let mut names: Vec<String> = std::fs::read_dir(self.project.join(STORE_DIR))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().join(rev).is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                names.sort();
                names
            }
        }
    }
}

/// A revision is a deployment hash: one folder name, so letters and digits.
fn valid_rev(rev: Option<&str>) -> Result<Option<&str>, FsError> {
    match rev {
        Some(rev) if rev.is_empty() || !rev.chars().all(|c| c.is_ascii_alphanumeric()) => Err(
            FsError::Invalid(format!("`{rev}` isn't a deployment hash for {EXTENSIONS}")),
        ),
        other => Ok(other),
    }
}

/// `path` as `(extension name, path inside it)`, both without `..`.
fn split(path: &str) -> Result<(String, String), FsError> {
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {
                return Err(FsError::Invalid(format!(
                    "`{path}` isn't a path inside {EXTENSIONS}"
                )))
            }
        }
    }
    let mut parts = parts.into_iter();
    let name = parts.next().unwrap_or_default();
    Ok((name, parts.collect::<Vec<_>>().join("/")))
}

/// The canonical deployment folder, and the canonical `inside` in it:
/// `None` when either isn't there, a refusal when `inside` leaves it.
fn canonical(dir: &Path, inside: &str) -> Result<Option<(PathBuf, PathBuf)>, FsError> {
    let Ok(root) = dir.canonicalize() else {
        return Ok(None);
    };
    let Ok(target) = root.join(inside).canonicalize() else {
        return Ok(None);
    };
    if target.starts_with(&root) {
        Ok(Some((root, target)))
    } else {
        Err(FsError::Invalid(format!(
            "`{inside}` leaves the deployment folder"
        )))
    }
}

fn storage(e: std::io::Error) -> FsError {
    FsError::Storage(e.to_string())
}

async fn blocking<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, FsError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| FsError::Storage(format!("file read didn't finish: {e}")))
}

fn dir_entry(path: String) -> FsEntry {
    FsEntry {
        path,
        kind: FsKind::Dir,
        status: None,
    }
}

/// Every entry below `dir`, as `prefix/…` paths: folders and files. A
/// symlink is skipped (it could lead out of the deployment).
fn walk(dir: &Path, prefix: &str, deep: bool, out: &mut Vec<FsEntry>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let kind = entry.file_type()?;
        let path = format!("{prefix}/{name}");
        if kind.is_dir() {
            out.push(dir_entry(path.clone()));
            if deep {
                walk(&entry.path(), &path, true, out)?;
            }
        } else if kind.is_file() {
            out.push(FsEntry {
                path,
                kind: FsKind::File,
                status: None,
            });
        }
    }
    Ok(())
}

#[async_trait]
impl FileSystem for ExtensionsFs {
    fn id(&self) -> &str {
        EXTENSIONS
    }

    fn features(&self) -> &[&'static str] {
        &[]
    }

    async fn stat(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
    ) -> Result<Option<FsStat>, FsError> {
        let rev = valid_rev(rev)?;
        let (name, _) = split(path)?;
        if name.is_empty() {
            return Ok(Some(FsStat {
                kind: FsKind::Dir,
                size: None,
                modified_at: None,
                is_text: None,
                language: None,
            }));
        }
        let Some(target) = self.target(rev, path)? else {
            return Ok(None);
        };
        let dir = self.dir_of(&target);
        blocking(move || {
            let Some((root, found)) = canonical(&dir, &target.inside)? else {
                return Ok(None);
            };
            let inside = found.strip_prefix(&root).unwrap_or(Path::new(""));
            stat_on_disk(&root, &inside.to_string_lossy()).map_err(storage)
        })
        .await?
    }

    async fn read(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<Vec<u8>>, FsError> {
        let Some(target) = self.target(rev, path)? else {
            return Ok(None);
        };
        let dir = self.dir_of(&target);
        blocking(move || {
            let Some((root, found)) = canonical(&dir, &target.inside)? else {
                return Ok(None);
            };
            if !found.is_file() {
                return Ok(None);
            }
            match range {
                Some(range) => {
                    let inside = found.strip_prefix(&root).unwrap_or(Path::new(""));
                    read_range_on_disk(&root, &inside.to_string_lossy(), range).map_err(storage)
                }
                None => std::fs::read(&found).map(Some).map_err(storage),
            }
        })
        .await?
    }

    async fn list(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<FsEntry>, FsError> {
        let rev = valid_rev(rev)?;
        if let Some(parent) = parent.filter(|p| !p.is_empty()) {
            let Some(target) = self.target(rev, parent)? else {
                return Ok(Vec::new());
            };
            let dir = self.dir_of(&target);
            return blocking(move || {
                let Some((_, found)) = canonical(&dir, &target.inside)? else {
                    return Ok(Vec::new());
                };
                let mut out = Vec::new();
                if found.is_dir() {
                    walk(&found, &parent_prefix(&target), false, &mut out).map_err(storage)?;
                }
                out.sort_by(|a, b| a.path.cmp(&b.path));
                Ok(out)
            })
            .await?;
        }
        let names = self.extension_names(rev);
        let mut out: Vec<FsEntry> = names.iter().cloned().map(dir_entry).collect();
        if parent.is_none() {
            // The whole tree: every extension's deployment below its name.
            let deployments: Vec<(String, PathBuf)> = names
                .into_iter()
                .filter_map(|name| {
                    let hash = rev.map(str::to_string).or_else(|| self.running(&name))?;
                    let dir = deployment_dir(&self.project, &name, &hash);
                    Some((name, dir))
                })
                .collect();
            let below = blocking(move || {
                let mut below = Vec::new();
                for (name, dir) in deployments {
                    if dir.is_dir() {
                        walk(&dir, &name, true, &mut below).map_err(storage)?;
                    }
                }
                Ok::<_, FsError>(below)
            })
            .await??;
            out.extend(below);
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

/// `<name>/<inside>` of a target: the prefix its children's paths have.
fn parent_prefix(target: &Target) -> String {
    if target.inside.is_empty() {
        target.name.clone()
    } else {
        format!("{}/{}", target.name, target.inside)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem_conformance as conformance;
    use crate::filesystem_conformance::ROOT;

    const HASH: &str = "abc123";

    /// A project whose extension `conformance-files` runs deployment
    /// [`HASH`], holding the conformance fixture.
    fn project() -> (tempfile::TempDir, ExtensionsFs) {
        let dir = tempfile::tempdir().unwrap();
        for (path, content) in conformance::fixture() {
            let (name, inside) = path.split_once('/').unwrap();
            let file = deployment_dir(dir.path(), name, HASH).join(inside);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, content).unwrap();
        }
        let catalog = Arc::new(ExtensionCatalog::for_project(dir.path()));
        catalog.set_live([(ROOT.to_string(), HASH.to_string())].into());
        let fs = ExtensionsFs::new(dir.path().to_path_buf(), catalog);
        (dir, fs)
    }

    fn cx() -> FsCx {
        FsCx::default()
    }

    fn write_deployment(dir: &Path, name: &str, hash: &str, file: &str, content: &str) {
        let path = deployment_dir(dir, name, hash).join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    async fn run_all(fs: &dyn FileSystem, rev: Option<&str>) {
        conformance::stat_describes_what_read_returns(fs, &cx(), rev).await;
        conformance::read_round_trips_and_ranges_slice(fs, &cx(), rev).await;
        conformance::lists_one_level_or_the_whole_tree(fs, &cx(), rev).await;
        conformance::a_path_never_leaves_the_filesystem(fs, &cx(), rev).await;
        conformance::writes_only_when_writable(fs, &cx(), rev).await;
    }

    #[tokio::test]
    async fn it_passes_the_conformance_checks_at_the_running_copy_and_by_hash() {
        let (_d, fs) = project();
        run_all(&fs, None).await;
        run_all(&fs, Some(HASH)).await;
    }

    #[tokio::test]
    async fn no_revision_is_the_running_deployment_and_a_hash_is_that_one() {
        let (dir, fs) = project();
        write_deployment(dir.path(), ROOT, "old456", "a.txt", "older\n");
        let a = format!("{ROOT}/a.txt");
        let cx = cx();
        let read = |rev| fs.read(&cx, rev, &a, None);
        assert_eq!(read(None).await.unwrap().unwrap(), b"one\ntwo\nthree\n");
        assert_eq!(
            read(Some(HASH)).await.unwrap().unwrap(),
            b"one\ntwo\nthree\n"
        );
        assert_eq!(read(Some("old456")).await.unwrap().unwrap(), b"older\n");
        assert_eq!(read(Some("nosuch")).await.unwrap(), None, "unknown hash");
    }

    #[tokio::test]
    async fn an_unknown_extension_or_a_file_that_is_not_there_is_none() {
        let (_d, fs) = project();
        for path in ["nope/a.txt", "nope", &format!("{ROOT}/missing.txt")] {
            assert_eq!(fs.stat(&cx(), None, path).await.unwrap(), None, "{path}");
            assert_eq!(
                fs.read(&cx(), None, path, None).await.unwrap(),
                None,
                "{path}"
            );
        }
        assert!(fs.list(&cx(), None, Some("nope")).await.unwrap().is_empty());
    }

    /// An extension that runs no deployment (here: not in the live set, so
    /// it loads from nothing) has no files without a revision, and isn't
    /// listed; one of its deployments is read by hash.
    #[tokio::test]
    async fn an_extension_that_runs_no_deployment_has_no_running_files() {
        let (dir, fs) = project();
        write_deployment(dir.path(), "idle", "h1", "x.txt", "x");
        assert_eq!(
            fs.read(&cx(), None, "idle/x.txt", None).await.unwrap(),
            None
        );
        let root = fs.list(&cx(), None, Some("")).await.unwrap();
        assert!(!root.iter().any(|e| e.path == "idle"), "{root:?}");
        assert!(root.iter().any(|e| e.path == ROOT));
        assert_eq!(
            fs.read(&cx(), Some("h1"), "idle/x.txt", None)
                .await
                .unwrap()
                .unwrap(),
            b"x"
        );
        let at_hash = fs.list(&cx(), Some("h1"), Some("")).await.unwrap();
        assert_eq!(
            at_hash.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["idle"]
        );
    }

    #[tokio::test]
    async fn a_revision_is_one_folder_name() {
        let (_d, fs) = project();
        for rev in ["", "../x", "a/b", "..", "a.b"] {
            let e = fs.read(&cx(), Some(rev), "x/y", None).await.unwrap_err();
            assert!(matches!(e, FsError::Invalid(_)), "{rev:?}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_out_of_a_deployment_is_refused() {
        let (dir, fs) = project();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        let deployment = deployment_dir(dir.path(), ROOT, HASH);
        std::os::unix::fs::symlink(&outside, deployment.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(dir.path(), deployment.join("up")).unwrap();
        let invalid = |e: FsError| matches!(e, FsError::Invalid(_));
        let link = format!("{ROOT}/link.txt");
        assert!(fs
            .read(&cx(), None, &link, None)
            .await
            .err()
            .is_some_and(invalid));
        assert!(fs.stat(&cx(), None, &link).await.err().is_some_and(invalid));
        let up = format!("{ROOT}/up");
        assert!(fs
            .list(&cx(), None, Some(&up))
            .await
            .err()
            .is_some_and(invalid));
        let tree = fs.list(&cx(), None, None).await.unwrap();
        assert!(
            !tree
                .iter()
                .any(|e| e.path.ends_with("link.txt") || e.path.contains("/up")),
            "a walk doesn't follow or list a link"
        );
    }

    #[tokio::test]
    async fn the_running_hash_is_the_one_the_catalog_names() {
        let (_d, fs) = project();
        let extensions = fs.catalog.unpinned(&fs.project, "a test");
        assert_eq!(running_hash(&extensions, ROOT), Some(HASH));
        assert_eq!(running_hash(&extensions, "nope"), None);
    }
}
