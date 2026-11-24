//! `storage:` and `files:` in `extension.yaml`: the extension's own SQLite
//! file and the project folders it may read or write
//! (`.context/extensions-permissions.md` "Store services"). What the host
//! does with them is `extension_services`.

use oxplow_domain::diagnostic::Diagnostic;
use oxplow_domain::folder_root::FolderRoot;
use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, key_line};
use super::ExtensionFiles;

/// Where an extension's migrations are, inside its folder.
pub const MIGRATIONS_DIR: &str = "storage/migrations";

/// `storage:` as written.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageFile {
    /// Give the extension an SQLite file of its own,
    /// `.oxplow/extensions-data/<name>.sqlite`, migrated by
    /// `storage/migrations/V<n>__<name>.sql` and reached with the
    /// `store.sql` service (`needs: [store.sql]`).
    #[serde(default)]
    pub sqlite: bool,
}

/// What a folder of `files:` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FolderAccess {
    Read,
    Write,
}

/// A `files:` entry as written.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FolderFile {
    /// A folder relative to its `root`, without `..`.
    pub path: String,
    /// What `path` is relative to: the project (the default), the person's
    /// home folder (`home`) or an agent harness's own data folder for this
    /// project (`agent:<harness id>`). A folder outside the project is read
    /// only: oxplow observes another tool's files, it never writes them.
    #[serde(default)]
    pub root: FolderRoot,
    /// `read` for `files.read`; `write` also allows `files.write`.
    pub access: FolderAccess,
    /// A folder oxplow watches, so the extension's `files` trigger hears
    /// it. In the project it is one folder directly under `.oxplow/`, not
    /// one of oxplow's own, which oxplow also snapshots though git ignores
    /// it: the extension's own store of files, as oxwiki's pages are.
    /// Outside the project any folder may be watched.
    #[serde(default)]
    pub watch: bool,
}

/// A folder the extension may use, its `path` relative to its `root`
/// without a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub path: String,
    #[serde(default)]
    pub root: FolderRoot,
    pub access: FolderAccess,
    /// See [`FolderFile::watch`].
    #[serde(default)]
    pub watch: bool,
}

/// What an extension declares for the store and file services. Host-side
/// only: it is left out of the UI's types.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtensionStorage {
    /// `storage: { sqlite: true }`.
    pub sqlite: bool,
    /// Its migrations, `(file name, sql)`, as read when it loaded.
    pub migrations: Vec<(String, String)>,
    pub folders: Vec<Folder>,
}

/// The folders directly under `.oxplow/` that are oxplow's own: no extension
/// may declare one as its watched folder.
const OXPLOW_OWN_DIRS: &[&str] = &[
    "extensions",
    "extensions-data",
    "runtime",
    "logs",
    "snapshots",
    "blobs",
    "objects",
    "lsp",
    "lsp-cache",
    "global-config",
];

/// Why no extension may declare (or reach) `path`: `.git` at any depth, in
/// any case (macOS is case-insensitive), is git's; `.oxplow` is oxplow's own
/// data (other extensions' store files in `extensions-data`, the extensions'
/// sources in `extensions`, the database ...) — except, when `watched`, a
/// folder directly under it that an extension declares `watch: true`
/// ([`watched_problem`]); `.git` is git's there too. `path` is already a
/// valid path.
pub(crate) fn reserved(path: &str, watched: bool) -> Option<&'static str> {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if parts.iter().any(|p| p.eq_ignore_ascii_case(".git")) {
        return Some("`.git` is git's");
    }
    if !watched && parts[0].eq_ignore_ascii_case(".oxplow") {
        return Some(
            "`.oxplow` is oxplow's own data (declare a folder under it with `watch: true`)",
        );
    }
    None
}

/// Why `path` can't be a watched folder: it is one folder directly under
/// `.oxplow/`, not one of oxplow's own, and not `.git`'s.
pub(crate) fn watched_problem(path: &str) -> Option<&'static str> {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if parts.iter().any(|p| p.eq_ignore_ascii_case(".git")) {
        return Some("`.git` is git's");
    }
    if parts.len() != 2 || parts[0] != ".oxplow" {
        return Some("a watched folder is one folder directly under `.oxplow/`");
    }
    if OXPLOW_OWN_DIRS
        .iter()
        .any(|d| d.eq_ignore_ascii_case(parts[1]))
    {
        return Some("that folder of `.oxplow/` is oxplow's own");
    }
    None
}

/// The names (under `.oxplow/`) of the folders the enabled `extensions`
/// declare watched: what the file watch and the snapshots follow.
pub(crate) fn watched_names(extensions: &[super::Extension]) -> Vec<String> {
    let mut names: Vec<String> = extensions
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.storage.folders.iter())
        .filter(|f| f.watch && f.root.is_project())
        .filter_map(|f| f.path.strip_prefix(".oxplow/").map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The folders outside the project, as their permission names them
/// (`agent:claude/memory`).
pub(crate) fn outside_folders(folders: &[Folder]) -> Vec<String> {
    folders
        .iter()
        .filter(|f| !f.root.is_project())
        .map(|f| format!("{}/{}", f.root, f.path))
        .collect()
}

/// Whether `path` (of the project) lies in a declared watched folder.
pub(crate) fn in_watched(folders: &[Folder], path: &str) -> bool {
    folders.iter().any(|f| {
        f.watch
            && f.root.is_project()
            && (path == f.path || path.starts_with(&format!("{}/", f.path)))
    })
}

/// Read and check the two keys. `files` is the extension's own folder, for
/// its migrations.
pub(crate) fn load(
    storage: Option<&StorageFile>,
    folders: &[FolderFile],
    files: &dyn ExtensionFiles,
    man: &str,
    manifest: &str,
) -> (ExtensionStorage, Vec<Diagnostic>) {
    let mut out = ExtensionStorage {
        sqlite: storage.is_some_and(|s| s.sqlite),
        ..Default::default()
    };
    let mut errors = Vec::new();
    let line = key_line(manifest, "files");
    for folder in folders {
        let path = folder.path.trim_end_matches('/');
        let outside = !folder.root.is_project();
        let problem = if !oxplow_domain::service::valid_path(path) {
            Some("a folder is relative to its root, with no `..`, `.` or empty parts")
        } else if outside && folder.access == FolderAccess::Write {
            Some("a folder outside the project is read only (`access: read`)")
        } else if let Some(why) = if outside {
            // Another tool's folder: only git's own is closed.
            reserved(path, true)
        } else if folder.watch {
            watched_problem(path)
        } else {
            reserved(path, false)
        } {
            Some(why)
        } else if out
            .folders
            .iter()
            .any(|f| f.path == path && f.root == folder.root)
        {
            Some("it is declared twice")
        } else {
            None
        };
        match problem {
            Some(why) => errors.push(at(
                "files.invalid_folder",
                man,
                line,
                format!("`files:` folder `{}`: {why}", folder.path),
            )),
            None => out.folders.push(Folder {
                path: path.to_string(),
                root: folder.root.clone(),
                access: folder.access,
                watch: folder.watch,
            }),
        }
    }
    let names = files.list(MIGRATIONS_DIR);
    if out.sqlite {
        for name in names {
            let Some(sql) = files.read(&format!("{MIGRATIONS_DIR}/{name}")) else {
                continue;
            };
            if let Err(why) = oxplow_db::extension_store::check_migration_name(&name) {
                errors.push(at(
                    "storage.invalid_migration",
                    man,
                    key_line(manifest, "storage"),
                    why,
                ));
                continue;
            }
            out.migrations.push((name, sql));
        }
    } else if !names.is_empty() {
        errors.push(at(
            "storage.migrations_without_storage",
            man,
            key_line(manifest, "storage"),
            format!(
                "`{MIGRATIONS_DIR}` holds files but `storage: {{ sqlite: true }}` isn't declared"
            ),
        ));
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use super::super::Tree;
    use super::*;

    fn load_tree(manifest: &str, extra: &[(&str, &str)]) -> super::super::Extension {
        let mut files = vec![(
            "extension.yaml".to_string(),
            format!("manifest: 2\nname: acme\nintent:\n  purpose: p\n{manifest}"),
        )];
        files.extend(extra.iter().map(|(p, t)| (p.to_string(), t.to_string())));
        Tree::new(files).load("acme", ".oxplow/extensions/acme")
    }

    #[test]
    fn a_folder_in_git_or_oxplows_own_data_is_refused() {
        for path in [
            ".git",
            ".GIT",
            ".Git/hooks",
            "sub/.git",
            "a/.GiT/b",
            ".oxplow",
            ".oxplow/extensions-data",
            ".oxplow/extensions",
            ".oxplow/extensions/x",
            ".OXPLOW/extensions-data",
            ".oxplow/oxwiki/../extensions",
            ".oxplow/oxwikix",
            ".oxplow/oxwiki2/a",
            // Without `watch: true` nothing under `.oxplow` is open.
            ".oxplow/oxwiki",
            ".oxplow/oxwiki/pages",
        ] {
            let ext = load_tree(
                &format!("files:\n  - {{ path: '{path}', access: write }}\n"),
                &[],
            );
            assert!(ext.storage.folders.is_empty(), "{path} was kept");
        }
        for path in ["gitless", "a/.github"] {
            let ext = load_tree(
                &format!("files:\n  - {{ path: '{path}', access: write }}\n"),
                &[],
            );
            assert_eq!(ext.storage.folders.len(), 1, "{path} was refused");
        }
    }

    /// A folder directly under `.oxplow/` is declared watched with `watch:
    /// true`; nothing else under `.oxplow` is, and oxplow's own folders are
    /// not.
    #[test]
    fn a_watched_folder_is_one_folder_under_oxplow_that_is_not_oxplows() {
        for (path, kept) in [
            (".oxplow/oxwiki", true),
            (".oxplow/notes/", true),
            (".oxplow/extensions", false),
            (".oxplow/extensions-data", false),
            (".oxplow/logs", false),
            (".OXPLOW/Runtime", false),
            (".oxplow", false),
            (".oxplow/a/b", false),
            ("docs", false),
            (".oxplow/.git", false),
        ] {
            let ext = load_tree(
                &format!("files:\n  - {{ path: '{path}', access: write, watch: true }}\n"),
                &[],
            );
            assert_eq!(ext.storage.folders.len(), usize::from(kept), "{path}");
            if kept {
                assert!(ext.storage.folders[0].watch);
            }
        }
    }

    /// The folders the enabled extensions declare watched reach the file
    /// watch's filter when a generation is activated, and leave with the
    /// extension.
    #[tokio::test]
    async fn an_activated_watched_folder_reaches_the_file_watch() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let write = |rel: &str, body: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: p\nfiles:\n  - { path: .oxplow/jotter, access: write, watch: true }\n",
        );
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let problems = crate::test_fixtures::activate_with_problems(&svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        let filter = oxplow_fs_watch::WorkspaceFilter::with_user_entries(Vec::<String>::new());
        let page = std::path::Path::new(".oxplow/jotter/a.md");
        assert!(!filter.ignore(page, false), "declared, so watched");
        std::fs::write(
            dir.path().join(".oxplow/local.yaml"),
            "oxplow.extensions.acme.enabled: false\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&svc).await;
        assert!(filter.ignore(page, false), "off with the extension");
    }

    /// A folder outside the project is read only, may be watched wherever
    /// it is, keeps git's reservation, and is another folder than the same
    /// path in the project. It never reaches the project's file watch.
    #[test]
    fn a_folder_outside_the_project_is_read_only() {
        let ext = load_tree(
            "files:\n  - { root: 'agent:claude', path: memory, access: read, watch: true }\n  - { path: memory, access: read }\n  - { root: home, path: Notes/vault, access: read }\n",
            &[],
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.storage.folders[0],
            Folder {
                path: "memory".into(),
                root: FolderRoot::Agent("claude".into()),
                access: FolderAccess::Read,
                watch: true,
            }
        );
        assert_eq!(ext.storage.folders.len(), 3);
        assert!(watched_names(std::slice::from_ref(&ext)).is_empty());
        // Each is a permission a person approves by name.
        assert_eq!(
            ext.permissions.folders,
            ["agent:claude/memory", "home/Notes/vault"]
        );
        assert_eq!(
            ext.permissions.added_over(&Default::default()),
            ["files — may read agent:claude/memory, home/Notes/vault outside the project"]
        );
        for (entry, needle) in [
            ("{ root: home, path: notes, access: write }", "read only"),
            ("{ root: home, path: a/.git, access: read }", ".git"),
            (
                "{ root: home, path: ../x, access: read }",
                "relative to its root",
            ),
            (
                "{ root: 'agent:Bad', path: x, access: read }",
                "folder root",
            ),
        ] {
            let ext = load_tree(&format!("files:\n  - {entry}\n"), &[]);
            let all = format!("{:?}", ext.errors);
            assert!(ext.storage.folders.is_empty(), "{entry} was kept");
            assert!(all.contains(needle), "{entry}: {all}");
        }
    }

    #[test]
    fn the_run_time_path_check_refuses_the_same_places() {
        let f = [Folder {
            path: ".oxplow/oxwiki".into(),
            root: Default::default(),
            access: FolderAccess::Write,
            watch: true,
        }];
        // Even a folder that slipped through never opens these.
        let all = [
            Folder {
                path: ".GIT".into(),
                root: Default::default(),
                access: FolderAccess::Write,
                watch: false,
            },
            Folder {
                path: "sub".into(),
                root: Default::default(),
                access: FolderAccess::Write,
                watch: false,
            },
            Folder {
                path: ".oxplow".into(),
                root: Default::default(),
                access: FolderAccess::Write,
                watch: false,
            },
        ];
        for path in [
            ".GIT/config",
            "sub/.git/config",
            "sub/.Git",
            ".oxplow/extensions-data/x.sqlite",
        ] {
            assert!(
                crate::extension::extension_services::check_path(&all, path, true).is_err(),
                "{path}"
            );
        }
        assert!(
            crate::extension::extension_services::check_path(&f, ".oxplow/oxwiki/a.md", true)
                .is_ok()
        );
    }

    #[test]
    fn storage_and_files_load_with_their_migrations() {
        let ext = load_tree(
            "storage: { sqlite: true }\nfiles:\n  - { path: shelf/, access: write }\n  - { path: docs, access: read }\n",
            &[
                ("storage/migrations/V1__book.sql", "CREATE TABLE b (x);"),
                ("storage/migrations/V2__more.sql", "SELECT 1;"),
            ],
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert!(ext.storage.sqlite);
        assert_eq!(ext.storage.migrations.len(), 2);
        assert_eq!(
            ext.storage.folders,
            vec![
                Folder {
                    path: "shelf".into(),
                    root: Default::default(),
                    access: FolderAccess::Write,
                    watch: false
                },
                Folder {
                    path: "docs".into(),
                    root: Default::default(),
                    access: FolderAccess::Read,
                    watch: false
                },
            ]
        );
    }

    /// A SQLite trigger names the extension's own file (needs `storage:`)
    /// or a file in a `files:` folder it may read; any other is an error
    /// at the collector's line.
    #[test]
    fn a_sqlite_trigger_stays_in_what_the_extension_may_read() {
        let collector = |on: &str| {
            format!(
                "permissions: [files.read]\ncollectors:\n  - id: tracker\n    runtime: starlark\n    entry: t.star\n    on: [{on}]\n    entities:\n      - {{ name: x, key: id, columns: {{ id: int }} }}\n"
            )
        };
        let script = ("t.star", "def transform(x):\n    return {}\n");
        let good = load_tree(
            &format!(
                "storage: {{ sqlite: true }}\nfiles:\n  - {{ path: .beads, access: read }}\n{}",
                collector("{ sqlite: own }, { sqlite: .beads/beads.db }")
            ),
            &[script],
        );
        assert!(good.errors.is_empty(), "{:?}", good.errors);
        assert_eq!(good.collectors.len(), 1);
        for (manifest, needle) in [
            (
                collector("{ sqlite: own }"),
                "needs `storage: { sqlite: true }`",
            ),
            (
                format!(
                    "files:\n  - {{ path: docs, access: read }}\n{}",
                    collector("{ sqlite: .beads/beads.db }")
                ),
                "outside every declared folder",
            ),
        ] {
            let ext = load_tree(&manifest, &[script]);
            let found = ext
                .errors
                .iter()
                .find(|e| e.code == "collector.trigger_invalid")
                .unwrap_or_else(|| panic!("{manifest}: {:?}", ext.errors));
            assert!(found.message.contains(needle), "{}", found.message);
            assert!(found.line.is_some(), "an error at file:line");
            assert!(ext.collectors.is_empty());
        }
    }

    #[test]
    fn a_bad_folder_or_migration_is_an_error() {
        for (manifest, extra, needle) in [
            (
                "files:\n  - { path: /abs, access: read }\n",
                vec![],
                "files:",
            ),
            (
                "files:\n  - { path: a/../b, access: read }\n",
                vec![],
                "files:",
            ),
            ("files:\n  - { path: .git, access: read }\n", vec![], ".git"),
            (
                "files:\n  - { path: a, access: read }\n  - { path: a, access: write }\n",
                vec![],
                "twice",
            ),
            (
                "storage: { sqlite: true }\n",
                vec![("storage/migrations/one.sql", "")],
                "V<n>__<name>.sql",
            ),
            (
                "",
                vec![("storage/migrations/V1__a.sql", "")],
                "isn't declared",
            ),
        ] {
            let ext = load_tree(manifest, &extra);
            let all = ext
                .errors
                .iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(all.contains(needle), "{manifest}: {all}");
        }
    }
}
