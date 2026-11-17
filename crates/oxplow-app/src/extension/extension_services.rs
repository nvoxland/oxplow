//! The store and file services (`.context/extensions-permissions.md`
//! "Store services"): `store.sql` on an extension's own SQLite file and
//! `files.read` / `files.list` / `files.write` / `files.delete` on the
//! project folders its manifest declares. The owner is always derived from
//! the run (`StateHost::with_owner`), never from the request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oxplow_db::extension_store::{ExtensionStore, Statement};
use oxplow_domain::folder_root::{FolderBases, FolderRoot};
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
    /// Where each root of a `files:` folder is (by default only the
    /// project's: [`Self::with_roots`] adds the ones outside it).
    roots: Arc<dyn FolderBases>,
    stores: Mutex<HashMap<String, Opened>>,
    /// What `links.check` looks links up in; none: the service is off.
    links: Option<crate::link_check::LinkDeps>,
}

impl ExtensionServices {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        let project = catalog.project_of(&project).to_path_buf();
        Self {
            owner: StateHost::new(catalog, project.clone()),
            roots: Arc::new(project.clone()),
            project,
            stores: Mutex::default(),
            links: None,
        }
    }

    /// Reach `files:` folders outside the project where `roots` says they are.
    pub fn with_roots(mut self, roots: Arc<dyn FolderBases>) -> Self {
        self.roots = roots;
        self
    }

    fn bases(&self) -> &dyn FolderBases {
        self.roots.as_ref()
    }

    /// Serve `links.check` over `links`.
    pub fn with_links(mut self, links: crate::link_check::LinkDeps) -> Self {
        self.links = Some(links);
        self
    }

    /// `links.check { body, self? }`: the links in `body` that don't resolve,
    /// as `{ warnings: [{ target, reason }] }`. Files are looked for in the
    /// run actor's worktree (the primary checkout when it has none); `self`
    /// is the URI of the page being written, which may link to itself. A
    /// script call runs on a blocking thread of the runtime, which this
    /// waits on.
    fn links_check(&self, run: &RunContext, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            body: String,
            #[serde(default, rename = "self")]
            this: Option<String>,
        }
        let Args { body, this } =
            serde_json::from_value(args).map_err(|_| "takes { body, self? }".to_string())?;
        let links = self.links.as_ref().ok_or("isn't available here")?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "needs the app's runtime".to_string())?;
        let anchors = run.anchors().clone();
        let root =
            runtime
                .block_on(links.db.read(move |conn| {
                    Ok(crate::link_check::worktree_of_anchors_tx(conn, &anchors))
                }))
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| links.project_dir.clone());
        let warnings = runtime
            .block_on(crate::link_check::check_links_at(
                &links.db,
                &links.vocabulary,
                &root,
                &*links.vcs,
                &body,
                this.as_deref(),
            ))
            .map_err(|e| e.to_string())?;
        Ok(json!({ "warnings": warnings }))
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
            "files.read" | "files.list" => files(self.bases(), &storage.folders, call, args),
            "links.check" => self.links_check(run, args),
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
            self.bases(),
            &storage.folders,
            "files.write",
            json!({ "path": path, "body": body }),
        )
    }

    /// `oxplow.extension_files.delete`.
    pub fn delete_file(&self, run: &RunContext, path: &str) -> Result<Value, String> {
        let (_, storage) = self.file_composer(run, "oxplow.extension_files.delete")?;
        files(
            self.bases(),
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
    check_root_path(folders, &FolderRoot::Project, path, write)
}

/// [`check_path`] for a path relative to `root`: only that root's folders
/// hold it, and one outside the project is never written.
pub fn check_root_path(
    folders: &[Folder],
    root: &FolderRoot,
    path: &str,
    write: bool,
) -> Result<String, String> {
    let refuse = |why: &str| {
        Err(format!(
            "path `{path}` {why}; declared folders: {}",
            declared(folders)
        ))
    };
    if path.starts_with('/') || path.contains(['\\', ':']) || path.chars().nth(1) == Some(':') {
        return refuse("is absolute; use a path relative to its root");
    }
    if write && !root.is_project() {
        return refuse(&format!(
            "is outside the project (`{root}`), which oxplow never writes"
        ));
    }
    let trimmed = path.trim_end_matches('/');
    if !oxplow_domain::service::valid_path(trimmed) {
        return refuse("has `..`, `.` or an empty part, or is empty");
    }
    // A watched folder lifts `.oxplow`'s reservation, never `.git`'s; a
    // folder outside the project has no `.oxplow` of oxplow's.
    let watched =
        !root.is_project() || crate::extension::extensions::storage::in_watched(folders, trimmed);
    if let Some(why) = crate::extension::extensions::storage::reserved(trimmed, watched) {
        return refuse(why);
    }
    let Some(folder) = folders.iter().find(|f| {
        &f.root == root && (trimmed == f.path || trimmed.starts_with(&format!("{}/", f.path)))
    }) else {
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

fn files(
    bases: &(impl FolderBases + ?Sized),
    folders: &[Folder],
    call: &str,
    args: Value,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        #[serde(default)]
        root: FolderRoot,
        path: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        glob: Option<String>,
        #[serde(default)]
        lossy: bool,
    }
    let takes = match call {
        "files.read" => "takes { path, root?, lossy? }",
        "files.delete" => "takes { path }",
        "files.list" => "takes { path, root?, glob? }",
        _ => "takes { path, body }",
    };
    let args: Args = serde_json::from_value(args).map_err(|e| format!("{takes}: {e}"))?;
    let write = matches!(call, "files.write" | "files.delete");
    let rel = check_root_path(folders, &args.root, &args.path, write)?;
    // A root with no folder on this machine (a harness that keeps none):
    // nothing is in it.
    let Some(project) = bases.base(&args.root) else {
        return match call {
            "files.list" => Ok(json!({ "files": [] })),
            _ => Err(format!("`{rel}`: `{}` has no folder here", args.root)),
        };
    };
    let project = project.as_path();
    no_symlink(project, &rel)?;
    let full = project.join(&rel);
    match call {
        "files.read" => {
            let meta = std::fs::metadata(&full).map_err(|e| format!("`{rel}`: {e}"))?;
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                return Err(format!("`{rel}` isn't a file of at most 16 MiB"));
            }
            if args.lossy {
                // Bytes that aren't UTF-8 become U+FFFD, and say so, so a
                // script can skip the file rather than fail on it.
                let bytes = std::fs::read(&full).map_err(|e| format!("`{rel}`: {e}"))?;
                let body = String::from_utf8_lossy(&bytes);
                let lossy = matches!(body, std::borrow::Cow::Owned(_));
                return Ok(json!({ "body": body, "lossy": lossy }));
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
                root: Default::default(),
                access: FolderAccess::Write,
                watch: false,
            },
            Folder {
                path: "docs/notes".into(),
                root: Default::default(),
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

    /// A watched folder lifts the `.oxplow` reservation, never git's.
    #[test]
    fn a_watched_folder_keeps_gits_reservation() {
        let f = vec![Folder {
            path: ".oxplow/wiki".into(),
            root: Default::default(),
            access: FolderAccess::Write,
            watch: true,
        }];
        assert_eq!(
            check_path(&f, ".oxplow/wiki/a.md", true).unwrap(),
            ".oxplow/wiki/a.md"
        );
        for path in [".oxplow/wiki/.git/config", ".oxplow/wiki/.GIT/x"] {
            let error = check_path(&f, path, true).unwrap_err();
            assert!(error.contains("`.git` is git's"), "{error}");
        }
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

    /// A script checks the links in a body through `links.check`: the ones
    /// that don't resolve come back with why, the ones that do don't.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_checks_a_bodys_links() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let write = |rel: &str, body: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: p\ncommands:\n  - name: note.check\n    summary: The links of a body that do not resolve.\n    entry: commands/check.star\n    needs: [links.check]\n    access: read\n    input_schema: { type: object, required: [body], additionalProperties: false, properties: { body: { type: string } } }\n",
        );
        write(
            ".oxplow/extensions/acme/commands/check.star",
            "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"links.check\", {\"body\": x[\"input\"][\"body\"]})}\n",
        );
        write("src/there.rs", "fn main() {}\n");
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let problems = crate::test_fixtures::activate_with_problems(&svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        let out = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "acme.note.check",
                json!({ "body": "see [[src/there.rs]], [[src/gone.rs]] and [[#13]]" }),
                false,
            )
            .await
            .unwrap();
        let targets: Vec<&str> = out.result["result"]["warnings"]
            .as_array()
            .unwrap_or_else(|| panic!("{}", out.result))
            .iter()
            .map(|w| w["target"].as_str().unwrap())
            .collect();
        assert_eq!(targets, ["src/gone.rs", "#13"], "{}", out.result);
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

    /// Where a test's roots are: the project at `project`, every other root
    /// under `outside` (none for `agent:gone`).
    struct Bases {
        project: PathBuf,
        outside: PathBuf,
    }

    impl FolderBases for Bases {
        fn base(&self, root: &FolderRoot) -> Option<PathBuf> {
            match root {
                FolderRoot::Project => Some(self.project.clone()),
                FolderRoot::Agent(id) if id == "gone" => None,
                other => Some(self.outside.join(other.to_string().replace(':', "-"))),
            }
        }
    }

    /// A folder outside the project is read and listed under its own root,
    /// its paths relative to that root; it is never written, and a root with
    /// no folder here lists nothing.
    #[test]
    fn a_folder_outside_the_project_is_read_under_its_root() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let memory = outside.path().join("agent-claude/memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("a.md"), "remembered").unwrap();
        // The same path in the project is another folder.
        std::fs::create_dir_all(project.path().join("memory")).unwrap();
        std::fs::write(project.path().join("memory/b.md"), "project's").unwrap();
        let bases = Bases {
            project: project.path().to_path_buf(),
            outside: outside.path().to_path_buf(),
        };
        let agent = |id: &str| FolderRoot::Agent(id.into());
        let f = vec![
            Folder {
                path: "memory".into(),
                root: agent("claude"),
                access: FolderAccess::Read,
                watch: true,
            },
            Folder {
                path: "memory".into(),
                root: agent("gone"),
                access: FolderAccess::Read,
                watch: false,
            },
        ];
        let run = |call: &str, args: Value| files(&bases, &f, call, args);
        let listed = run(
            "files.list",
            json!({ "root": "agent:claude", "path": "memory", "glob": "*.md" }),
        )
        .unwrap();
        assert_eq!(listed["files"][0]["path"], "memory/a.md");
        assert_eq!(listed["files"].as_array().unwrap().len(), 1);
        let read = run(
            "files.read",
            json!({ "root": "agent:claude", "path": "memory/a.md" }),
        )
        .unwrap();
        assert_eq!(read["body"], "remembered");
        // No root: the project's, where no folder is declared.
        let error = run("files.read", json!({ "path": "memory/b.md" })).unwrap_err();
        assert!(error.contains("outside every declared folder"), "{error}");
        // A root with no folder on this machine has nothing in it.
        let none = run(
            "files.list",
            json!({ "root": "agent:gone", "path": "memory" }),
        )
        .unwrap();
        assert_eq!(none, json!({ "files": [] }));
        assert!(run(
            "files.read",
            json!({ "root": "agent:gone", "path": "memory/a.md" })
        )
        .is_err());
        // Never written, even if a read-write folder slipped through.
        let writable = vec![Folder {
            access: FolderAccess::Write,
            ..f[0].clone()
        }];
        let error = files(
            &bases,
            &writable,
            "files.write",
            json!({ "root": "agent:claude", "path": "memory/c.md", "body": "x" }),
        )
        .unwrap_err();
        assert!(error.contains("outside the project"), "{error}");
        assert!(!memory.join("c.md").exists());
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
