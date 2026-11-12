//! `storage:` and `files:` in `extension.yaml`: the extension's own SQLite
//! file and the project folders it may read or write
//! (`.context/extensions-permissions.md` "Store services"). What the host
//! does with them is `extension_services`.

use oxplow_domain::diagnostic::Diagnostic;
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
    /// A folder relative to the project root, without `..`.
    pub path: String,
    /// `read` for `files.read`; `write` also allows `files.write`.
    pub access: FolderAccess,
}

/// A folder the extension may use, its `path` relative to the project root
/// without a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub path: String,
    pub access: FolderAccess,
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

/// Why no extension may declare (or reach) `path`: `.git` at any depth, in
/// any case (macOS is case-insensitive), is git's; `.oxplow` is oxplow's own
/// data (other extensions' store files in `extensions-data`, the extensions'
/// sources in `extensions`, the database ...). The one exception is
/// `.oxplow/oxwiki`: oxwiki's markdown files are its store, so the extension
/// that holds them needs write access there. `path` is already a valid path.
pub(crate) fn reserved(path: &str) -> Option<&'static str> {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if parts.iter().any(|p| p.eq_ignore_ascii_case(".git")) {
        return Some("`.git` is git's");
    }
    if parts[0].eq_ignore_ascii_case(".oxplow") {
        let oxwiki = parts.len() >= 2 && parts[1] == "oxwiki";
        if !oxwiki {
            return Some("`.oxplow` is oxplow's own data (only `.oxplow/oxwiki` is open)");
        }
    }
    None
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
        let problem = if !oxplow_domain::service::valid_path(path) {
            Some("a folder is relative to the project root, with no `..`, `.` or empty parts")
        } else if let Some(why) = reserved(path) {
            Some(why)
        } else if out.folders.iter().any(|f| f.path == path) {
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
                access: folder.access,
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
        ] {
            let ext = load_tree(
                &format!("files:\n  - {{ path: '{path}', access: write }}\n"),
                &[],
            );
            assert!(ext.storage.folders.is_empty(), "{path} was kept");
        }
        for path in [
            ".oxplow/oxwiki",
            ".oxplow/oxwiki/pages",
            "gitless",
            "a/.github",
        ] {
            let ext = load_tree(
                &format!("files:\n  - {{ path: '{path}', access: write }}\n"),
                &[],
            );
            assert_eq!(ext.storage.folders.len(), 1, "{path} was refused");
        }
    }

    #[test]
    fn the_run_time_path_check_refuses_the_same_places() {
        let f = [Folder {
            path: ".oxplow/oxwiki".into(),
            access: FolderAccess::Write,
        }];
        // Even a folder that slipped through never opens these.
        let all = [
            Folder {
                path: ".GIT".into(),
                access: FolderAccess::Write,
            },
            Folder {
                path: "sub".into(),
                access: FolderAccess::Write,
            },
            Folder {
                path: ".oxplow".into(),
                access: FolderAccess::Write,
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
                    access: FolderAccess::Write
                },
                Folder {
                    path: "docs".into(),
                    access: FolderAccess::Read
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
