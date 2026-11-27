//! The projects the host knows: `host.db`'s `project` table, each
//! `{ id, path, name }`, in the order they were added.
//!
//! The running host holds `host.db`, so the CLI's `project add|list|remove`
//! go through it (`/host/projects`) while it runs, and open the file
//! themselves only when no host does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::HostDb;

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
    #[error("the projects store failed: {0}")]
    Store(#[from] rusqlite::Error),
}

pub struct Registry {
    db: Arc<HostDb>,
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
pub(crate) fn id_for(path: &Path, mut taken: impl FnMut(&str) -> bool) -> String {
    id_candidates(path)
        .find(|id| !taken(id))
        .expect("an endless iterator has a free id")
}

fn project_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        name: row.get(2)?,
    })
}

impl Registry {
    pub fn new(db: Arc<HostDb>) -> Self {
        Self { db }
    }

    pub fn list(&self) -> Vec<Project> {
        let conn = self.db.conn();
        let listed = conn
            .prepare("SELECT id, path, name FROM project ORDER BY rowid")
            .and_then(|mut q| q.query_map([], project_of)?.collect::<Result<Vec<_>, _>>());
        listed.unwrap_or_else(|e| {
            tracing::error!(error = %e, "cannot list the projects");
            Vec::new()
        })
    }

    pub fn get(&self, id: &str) -> Option<Project> {
        self.db
            .conn()
            .query_row(
                "SELECT id, path, name FROM project WHERE id = ?1",
                [id],
                project_of,
            )
            .optional()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "cannot read a project");
                None
            })
    }

    /// Add the project at `dir`, or return the entry it already has.
    pub fn add(&self, dir: &Path) -> Result<Project, RegistryError> {
        let path = dir
            .canonicalize()
            .map_err(|e| RegistryError::Unreadable(dir.to_path_buf(), e))?;
        if !path.join(".oxplow").is_dir() {
            return Err(RegistryError::NotAProject(path));
        }
        let text = path.to_string_lossy().into_owned();
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        let known = tx
            .query_row(
                "SELECT id, path, name FROM project WHERE path = ?1",
                [&text],
                project_of,
            )
            .optional()?;
        if let Some(known) = known {
            return Ok(known);
        }
        let id = {
            let mut taken = tx.prepare("SELECT 1 FROM project WHERE id = ?1")?;
            id_for(&path, |id| taken.exists([id]).unwrap_or(true))
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        tx.execute(
            "INSERT INTO project (id, path, name) VALUES (?1, ?2, ?3)",
            params![id, text, name],
        )?;
        tx.commit()?;
        Ok(Project { id, path, name })
    }

    pub fn remove(&self, id: &str) -> Result<Project, RegistryError> {
        self.db
            .conn()
            .query_row(
                "DELETE FROM project WHERE id = ?1 RETURNING id, path, name",
                [id],
                project_of,
            )
            .optional()?
            .ok_or_else(|| RegistryError::UnknownProject(id.to_string()))
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
        let reg = Registry::new(HostDb::open(&tmp.path().join("home")).unwrap());
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
        let reg = Registry::new(HostDb::open(&tmp.path().join("home")).unwrap());
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
        let reg = Registry::new(HostDb::open(&tmp.path().join("home")).unwrap());
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
    fn the_projects_are_kept_in_the_order_they_were_added() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let reg = Registry::new(HostDb::open(&home).unwrap());
        let zeta = reg.add(&project_dir(tmp.path(), "zeta")).unwrap();
        let alpha = reg.add(&project_dir(tmp.path(), "alpha")).unwrap();
        drop(reg);
        let again = Registry::new(HostDb::open(&home).unwrap());
        assert_eq!(again.list(), vec![zeta, alpha]);
    }
}
