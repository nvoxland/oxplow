//! The store and file services (`.context/extensions-permissions.md`
//! "Store services"): `store.sql` on an extension's own SQLite file and
//! `files.read` / `files.list` / `files.write` / `files.delete` on the
//! project folders its manifest declares. The owner is always derived from
//! the run (`StateHost::with_owner`), never from the request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oxplow_db::extension_store::{ExtensionStore, Statement};
use oxplow_domain::run::RunContext;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::extension::extension_catalog::ExtensionCatalog;
use crate::extension::extension_state::StateHost;
use crate::extension::extensions::storage::{ExtensionStorage, Folder, FolderAccess};

/// Where extensions' SQLite files live, under the project root.
pub const DATA_DIR: &str = ".oxplow/extensions-data";

/// The most a file read answers.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// `<project>/.oxplow/extensions-data/<name>.sqlite`.
pub fn store_path(project: &Path, name: &str) -> PathBuf {
    project.join(DATA_DIR).join(format!("{name}.sqlite"))
}

/// Delete an extension's SQLite file with the rest of its data (an
/// uninstall; never a disable).
pub fn remove_store_file(project: &Path, name: &str) {
    let path = store_path(project, name);
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.clone().into_os_string();
        file.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(file));
    }
}

struct Opened {
    migrations: Vec<(String, String)>,
    store: ExtensionStore,
}

/// The services' host: one per project, holding each extension's open file.
pub struct ExtensionServices {
    owner: StateHost,
    project: PathBuf,
    stores: Mutex<HashMap<String, Opened>>,
}

impl ExtensionServices {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        let project = catalog.project_of(&project).to_path_buf();
        Self {
            owner: StateHost::new(catalog, project.clone()),
            project,
            stores: Mutex::default(),
        }
    }

    /// Answer a read `call` (`store.sql`, `files.read`, `files.list`) for the
    /// extension `run` belongs to. Writes are commands, not calls
    /// ([`Self::write_store`], [`Self::write_file`], [`Self::delete_file`]).
    pub fn call(&self, run: &RunContext, call: &str, args: Value) -> Result<Value, String> {
        let need = oxplow_domain::service::need_of_call(call);
        let (name, storage) = self
            .owner
            .with_owner(run, need, |ext| (ext.name.clone(), ext.storage.clone()))
            .map_err(|e| format!("`{call}`: {e}"))?;
        let answer = match call {
            "store.sql" => self.store_read(&name, &storage, args),
            "files.read" | "files.list" => files(&self.project, &storage.folders, call, args),
            _ => Err("isn't a read".into()),
        }
        .map_err(|e| format!("`{call}`: {e}"))?;
        // A revoked or replaced extension publishes nothing.
        run.check_authority()?;
        Ok(answer)
    }

    /// `oxplow.extension_store.write`: the statements in one transaction on
    /// the file of the extension the run is composed by; nobody else.
    /// Answers `{ changes, rows }`: `rows` one list per statement of the
    /// rows it returned (`RETURNING`), `[]` for one that returns none.
    pub fn write_store(&self, run: &RunContext, batch: &[Statement]) -> Result<Value, String> {
        let (name, storage) = self.composer(run, "oxplow.extension_store.write")?;
        let answer = self
            .store(&name, &storage)?
            .write(batch)
            .map_err(|e| e.to_string())?;
        Ok(json!({ "changes": answer.changes, "rows": answer.rows }))
    }

    /// `oxplow.extension_files.write`.
    pub fn write_file(&self, run: &RunContext, path: &str, body: &str) -> Result<Value, String> {
        let (_, storage) = self.file_composer(run, "oxplow.extension_files.write")?;
        files(
            &self.project,
            &storage.folders,
            "files.write",
            json!({ "path": path, "body": body }),
        )
    }

    /// `oxplow.extension_files.delete`.
    pub fn delete_file(&self, run: &RunContext, path: &str) -> Result<Value, String> {
        let (_, storage) = self.file_composer(run, "oxplow.extension_files.delete")?;
        files(
            &self.project,
            &storage.folders,
            "files.delete",
            json!({ "path": path }),
        )
    }

    /// [`Self::composer`], also holding `permissions: [files.write]`.
    fn file_composer(
        &self,
        run: &RunContext,
        command: &str,
    ) -> Result<(String, ExtensionStorage), String> {
        let (name, storage, may) = self
            .owner
            .with_composer(run, |ext| {
                (
                    ext.name.clone(),
                    ext.storage.clone(),
                    ext.permissions.files == Some(oxplow_domain::permissions::FilesAccess::Write),
                )
            })
            .map_err(|_| {
                format!("`{command}` can only be composed by an extension's own script")
            })?;
        if !may {
            return Err(format!(
                "`{command}` needs `files.write` in the extension's `permissions:`"
            ));
        }
        Ok((name, storage))
    }

    /// The extension a write command is composed by: a person or an agent
    /// running it directly has none, and is refused.
    /// The name of the extension whose script composed `run`'s command.
    pub fn composer_name(&self, run: &RunContext, command: &str) -> Result<String, String> {
        self.owner
            .with_composer(run, |ext| ext.name.clone())
            .map_err(|_| format!("`{command}` can only be composed by an extension's own script"))
    }

    fn composer(
        &self,
        run: &RunContext,
        command: &str,
    ) -> Result<(String, ExtensionStorage), String> {
        self.owner
            .with_composer(run, |ext| (ext.name.clone(), ext.storage.clone()))
            .map_err(|_| format!("`{command}` can only be composed by an extension's own script"))
    }

    fn store(&self, name: &str, storage: &ExtensionStorage) -> Result<ExtensionStore, String> {
        if !storage.sqlite {
            return Err("declare `storage: { sqlite: true }` in extension.yaml to use it".into());
        }
        let path = store_path(&self.project, name);
        let mut stores = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        // An uninstall deletes the file under an open pool: open it afresh.
        if !path.exists() {
            stores.remove(name);
        }
        if let Some(opened) = stores.get(name) {
            if opened.migrations == storage.migrations {
                return Ok(opened.store.clone());
            }
        }
        let store = ExtensionStore::open(&path).map_err(|e| e.to_string())?;
        store
            .migrate(&storage.migrations)
            .map_err(|e| e.to_string())?;
        stores.insert(
            name.to_string(),
            Opened {
                migrations: storage.migrations.clone(),
                store: store.clone(),
            },
        );
        Ok(store)
    }

    fn store_read(
        &self,
        name: &str,
        storage: &ExtensionStorage,
        args: Value,
    ) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            sql: String,
            #[serde(default)]
            params: Vec<Value>,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| format!("takes {{ sql, params? }} (reads only): {e}"))?;
        let answer = self
            .store(name, storage)?
            .read(&Statement {
                sql: args.sql,
                params: args.params,
                expect_changes: None,
            })
            .map_err(|e| e.to_string())?;
        // One statement: its rows.
        let rows = answer.rows.into_iter().next().unwrap_or_default();
        Ok(json!({ "rows": rows }))
    }
}

/// The folders as one sentence, for a refusal.
fn declared(folders: &[Folder]) -> String {
    if folders.is_empty() {
        return "none declared (`files:` in extension.yaml)".into();
    }
    folders
        .iter()
        .map(|f| {
            let access = match f.access {
                FolderAccess::Read => "read",
                FolderAccess::Write => "write",
            };
            format!("{} ({access})", f.path)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `path` (relative to the project root) if it lies in a declared folder
/// allowing `write` when asked: a normalized `/`-separated path. Refuses an
/// absolute path, a `..`, an empty part, and a folder not declared — the
/// message names the path and the declared folders.
pub fn check_path(folders: &[Folder], path: &str, write: bool) -> Result<String, String> {
    let refuse = |why: &str| {
        Err(format!(
            "path `{path}` {why}; declared folders: {}",
            declared(folders)
        ))
    };
    if path.starts_with('/') || path.contains(['\\', ':']) || path.chars().nth(1) == Some(':') {
        return refuse("is absolute; use a path relative to the project root");
    }
    let trimmed = path.trim_end_matches('/');
    if !oxplow_domain::service::valid_path(trimmed) {
        return refuse("has `..`, `.` or an empty part, or is empty");
    }
    if !crate::extension::extensions::storage::in_watched(folders, trimmed) {
        if let Some(why) = crate::extension::extensions::storage::reserved(trimmed) {
            return refuse(why);
        }
    }
    let Some(folder) = folders
        .iter()
        .find(|f| trimmed == f.path || trimmed.starts_with(&format!("{}/", f.path)))
    else {
        return refuse("is outside every declared folder");
    };
    if write && folder.access != FolderAccess::Write {
        return refuse(&format!("is in `{}`, declared read-only", folder.path));
    }
    Ok(trimmed.to_string())
}

/// A symlink on the way would lead out of the declared folder: refuse it.
fn no_symlink(project: &Path, rel: &str) -> Result<(), String> {
    let mut at = project.to_path_buf();
    for part in rel.split('/') {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("`{rel}` goes through a symbolic link"))
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

fn files(project: &Path, folders: &[Folder], call: &str, args: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        path: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        glob: Option<String>,
    }
    let takes = match call {
        "files.read" | "files.delete" => "takes { path }",
        "files.list" => "takes { path, glob? }",
        _ => "takes { path, body }",
    };
    let args: Args = serde_json::from_value(args).map_err(|e| format!("{takes}: {e}"))?;
    let write = matches!(call, "files.write" | "files.delete");
    let rel = check_path(folders, &args.path, write)?;
    no_symlink(project, &rel)?;
    let full = project.join(&rel);
    match call {
        "files.read" => {
            let meta = std::fs::metadata(&full).map_err(|e| format!("`{rel}`: {e}"))?;
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                return Err(format!("`{rel}` isn't a file of at most 16 MiB"));
            }
            let body = std::fs::read_to_string(&full).map_err(|e| format!("`{rel}`: {e}"))?;
            Ok(json!({ "body": body }))
        }
        "files.list" => list(project, &rel, args.glob.as_deref()),
        "files.write" => {
            let body = args.body.ok_or_else(|| takes.to_string())?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("`{rel}`: {e}"))?;
            }
            // Whole or not at all: write beside, then move into place.
            let temp = full.with_extension(format!("oxplow-{}.tmp", std::process::id()));
            std::fs::write(&temp, body).map_err(|e| format!("`{rel}`: {e}"))?;
            std::fs::rename(&temp, &full).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                format!("`{rel}`: {e}")
            })?;
            Ok(json!({ "path": rel }))
        }
        _ => match std::fs::remove_file(&full) {
            Ok(()) => Ok(json!({ "deleted": true })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({ "deleted": false })),
            Err(e) => Err(format!("`{rel}`: {e}")),
        },
    }
}

fn list(project: &Path, rel: &str, glob: Option<&str>) -> Result<Value, String> {
    let matcher = glob
        .map(|g| {
            globset::Glob::new(g)
                .map(|g| g.compile_matcher())
                .map_err(|e| format!("glob `{g}`: {e}"))
        })
        .transpose()?;
    let root = project.join(rel);
    let mut found = Vec::new();
    if root.is_dir() {
        for entry in walkdir::WalkDir::new(&root).follow_links(false) {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().is_file() {
                continue;
            }
            let inside = entry
                .path()
                .strip_prefix(&root)
                .map_err(|e| e.to_string())?;
            if matcher.as_ref().is_some_and(|m| !m.is_match(inside)) {
                continue;
            }
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            let modified = meta
                .modified()
                .ok()
                .map(time::OffsetDateTime::from)
                .and_then(|t| {
                    t.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                });
            found.push(json!({
                "path": format!("{rel}/{}", inside.to_string_lossy().replace('\\', "/")),
                "size": meta.len(),
                "modified_at": modified,
            }));
        }
    }
    found.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(json!({ "files": found }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<Folder> {
        vec![
            Folder {
                path: "shelf".into(),
                access: FolderAccess::Write,
                watch: false,
            },
            Folder {
                path: "docs/notes".into(),
                access: FolderAccess::Read,
                watch: false,
            },
        ]
    }

    #[test]
    fn a_path_in_a_declared_folder_is_kept() {
        let f = folders();
        assert_eq!(check_path(&f, "shelf", true).unwrap(), "shelf");
        assert_eq!(
            check_path(&f, "shelf/a/b.md", true).unwrap(),
            "shelf/a/b.md"
        );
        assert_eq!(
            check_path(&f, "docs/notes/x.md", false).unwrap(),
            "docs/notes/x.md"
        );
    }

    #[test]
    fn a_refusal_names_the_path_and_the_declared_folders() {
        let f = folders();
        for (path, write) in [
            ("/etc/passwd", false),
            ("shelf/../secret", false),
            ("../shelf/a", false),
            ("shelf2/a", false),
            ("docs/a", false),
            ("src/main.rs", true),
            ("", false),
            ("C:/x", false),
            ("shelf//a", false),
            ("docs/notes/x.md", true),
        ] {
            let error = check_path(&f, path, write).unwrap_err();
            assert!(error.contains(&format!("`{path}`")), "{error}");
            assert!(
                error.contains("shelf (write), docs/notes (read)"),
                "{error}"
            );
        }
        assert!(check_path(&[], "a", false)
            .unwrap_err()
            .contains("none declared"));
    }

    #[test]
    fn files_are_written_listed_read_and_deleted_inside_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let f = folders();
        let run = |call: &str, args: Value| files(dir.path(), &f, call, args);
        run(
            "files.write",
            json!({ "path": "shelf/a/one.md", "body": "one" }),
        )
        .unwrap();
        run(
            "files.write",
            json!({ "path": "shelf/two.txt", "body": "22" }),
        )
        .unwrap();
        let all = run("files.list", json!({ "path": "shelf" })).unwrap();
        let paths: Vec<_> = all["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths, ["shelf/a/one.md", "shelf/two.txt"]);
        assert_eq!(all["files"][1]["size"], 2);
        assert!(all["files"][1]["modified_at"].is_string());
        let md = run("files.list", json!({ "path": "shelf", "glob": "**/*.md" })).unwrap();
        assert_eq!(md["files"].as_array().unwrap().len(), 1);
        let read = run("files.read", json!({ "path": "shelf/a/one.md" })).unwrap();
        assert_eq!(read, json!({ "body": "one" }));
        assert_eq!(
            run("files.delete", json!({ "path": "shelf/two.txt" })).unwrap(),
            json!({ "deleted": true })
        );
        assert_eq!(
            run("files.delete", json!({ "path": "shelf/two.txt" })).unwrap(),
            json!({ "deleted": false })
        );
        // Read-only and undeclared folders refuse a write; nothing lands.
        assert!(run("files.write", json!({ "path": "docs/notes/x", "body": "" })).is_err());
        assert!(!dir.path().join("docs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_does_not_lead_out_of_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("shelf")).unwrap();
        let error = files(
            dir.path(),
            &folders(),
            "files.read",
            json!({ "path": "shelf/secret" }),
        )
        .unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");
    }
}
