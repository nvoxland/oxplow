//! The projects the host knows: `host-projects.json` in the global config
//! dir, each `{ id, path, name }`.
//!
//! The CLI (`oxplow-host project add`) and a running host are different
//! processes, so every read goes to the file and every change happens under
//! a lock on a sidecar (never on the file, which a rename replaces). Writes
//! are atomic, so a reader sees the old list or the new one.

use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One project the host can start a daemon for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Short and stable (`prj-` and 8 hex digits of a hash of the path), so a
    /// URL or a hold never carries a path.
    pub id: String,
    /// The canonical path of the project folder.
    pub path: PathBuf,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(
        "{0} is not an oxplow project (it has no .oxplow/ folder); open it once in oxplow first"
    )]
    NotAProject(PathBuf),
    #[error("cannot read {0}: {1}")]
    Unreadable(PathBuf, std::io::Error),
    #[error("no project has the id {0}")]
    UnknownProject(String),
    #[error("the registry file failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    projects: Vec<Project>,
}

pub struct Registry {
    path: PathBuf,
}

/// The ids a path may take, best first: 8 hex digits of a hash of the path,
/// then of the path with a counter, so a clash with another project's id
/// moves on to the next one.
fn id_candidates(path: &Path) -> impl Iterator<Item = String> + '_ {
    (0u32..).map(move |n| {
        let mut hash = Sha256::new();
        hash.update(path.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(n.to_be_bytes());
        format!("prj-{}", hex::encode(&hash.finalize()[..4]))
    })
}

/// The id for `path`, skipping any that `taken` says another project has.
pub(crate) fn id_for(path: &Path, taken: impl Fn(&str) -> bool) -> String {
    id_candidates(path)
        .find(|id| !taken(id))
        .expect("an endless iterator has a free id")
}

impl Registry {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("host-projects.json"),
        }
    }

    pub fn list(&self) -> Vec<Project> {
        self.read()
    }

    pub fn get(&self, id: &str) -> Option<Project> {
        self.read().into_iter().find(|p| p.id == id)
    }

    /// Add the project at `dir`, or return the entry it already has.
    pub fn add(&self, dir: &Path) -> Result<Project, RegistryError> {
        let path = dir
            .canonicalize()
            .map_err(|e| RegistryError::Unreadable(dir.to_path_buf(), e))?;
        if !path.join(".oxplow").is_dir() {
            return Err(RegistryError::NotAProject(path));
        }
        self.change(|projects| {
            if let Some(known) = projects.iter().find(|p| p.path == path) {
                return Ok(known.clone());
            }
            let id = id_for(&path, |id| projects.iter().any(|p| p.id == id));
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| id.clone());
            let project = Project { id, path, name };
            projects.push(project.clone());
            Ok(project)
        })
    }

    pub fn remove(&self, id: &str) -> Result<Project, RegistryError> {
        self.change(|projects| {
            let at = projects
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| RegistryError::UnknownProject(id.to_string()))?;
            Ok(projects.remove(at))
        })
    }

    fn read(&self) -> Vec<Project> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RegistryFile>(&bytes).ok())
            .map(|f| f.projects)
            .unwrap_or_default()
    }

    /// Read, change and write the list under the lock.
    fn change<T>(
        &self,
        f: impl FnOnce(&mut Vec<Project>) -> Result<T, RegistryError>,
    ) -> Result<T, RegistryError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(oxplow_config::lock_path(&self.path))?;
        lock.lock_exclusive()?;
        let mut projects = self.read();
        let out = f(&mut projects)?;
        let bytes =
            serde_json::to_vec_pretty(&RegistryFile { projects }).map_err(std::io::Error::other)?;
        oxplow_config::write_atomic(&self.path, &bytes)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_dir(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        dir
    }

    #[test]
    fn a_project_is_added_listed_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::new(&tmp.path().join("home"));
        let dir = project_dir(tmp.path(), "alpha");
        let added = reg.add(&dir).unwrap();
        assert_eq!(added.name, "alpha");
        assert_eq!(added.path, dir.canonicalize().unwrap());
        assert!(
            added.id.starts_with("prj-") && added.id.len() == 12,
            "{}",
            added.id
        );
        assert_eq!(reg.list(), vec![added.clone()]);
        assert_eq!(reg.get(&added.id), Some(added.clone()));

        assert_eq!(reg.remove(&added.id).unwrap(), added);
        assert!(reg.list().is_empty());
        assert!(matches!(
            reg.remove(&added.id),
            Err(RegistryError::UnknownProject(_))
        ));
    }

    #[test]
    fn adding_a_project_twice_keeps_one_entry_and_its_id() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::new(tmp.path());
        let dir = project_dir(tmp.path(), "alpha");
        let first = reg.add(&dir).unwrap();
        // The same folder by another path (a trailing `.`) is the same project.
        let second = reg.add(&dir.join(".")).unwrap();
        assert_eq!(first, second);
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn a_folder_without_oxplow_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::new(tmp.path());
        let plain = tmp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(matches!(
            reg.add(&plain),
            Err(RegistryError::NotAProject(_))
        ));
        assert!(matches!(
            reg.add(&tmp.path().join("missing")),
            Err(RegistryError::Unreadable(..))
        ));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn the_id_comes_from_the_path_and_is_stable() {
        let a = id_for(Path::new("/work/a"), |_| false);
        assert_eq!(a, id_for(Path::new("/work/a"), |_| false));
        assert_ne!(a, id_for(Path::new("/work/b"), |_| false));
        assert!(!a.contains("work"), "the id is not the path");
    }

    #[test]
    fn a_taken_id_moves_to_the_next_candidate() {
        let path = Path::new("/work/a");
        let first = id_for(path, |_| false);
        let second = id_for(path, |id| id == first);
        assert_ne!(first, second);
        assert!(second.starts_with("prj-") && second.len() == 12);
    }

    #[test]
    fn a_second_registry_sees_what_the_first_wrote() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = project_dir(tmp.path(), "alpha");
        let added = Registry::new(tmp.path()).add(&dir).unwrap();
        assert_eq!(Registry::new(tmp.path()).list(), vec![added]);
    }
}
