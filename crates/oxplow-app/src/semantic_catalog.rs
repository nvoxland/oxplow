//! The full semantic-layer catalog: core `v_*` views plus every entity
//! extensions declare through sources. One place, so the IPC and MCP
//! `describe_schema` can't disagree. See `.context/semantic-layer.md`.

use std::path::Path;

use oxplow_db::{SchemaColumn, SchemaEntity, SchemaRelation, SqlCell};

use crate::extension_sources::ColumnType;
use oxplow_domain::DomainError;

/// Core entities first, then extension entities (owner = extension
/// name), read from `root/oxplow/extensions/`.
pub async fn describe_schema(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
) -> Result<Vec<SchemaEntity>, DomainError> {
    let mut all = layer.describe_schema().await?;
    let existing = layer.view_names().await?;
    for ext in catalog.get(root).iter() {
        for source in &ext.sources {
            for e in &source.entities {
                all.push(SchemaEntity {
                    name: e.view.clone(),
                    description: if e.doc.is_empty() {
                        format!(
                            "`{}` records from the `{}` source of extension `{}`.",
                            e.name, source.id, ext.name
                        )
                    } else {
                        e.doc.clone()
                    },
                    owner: ext.name.clone(),
                    columns: e
                        .columns
                        .iter()
                        .map(|c| SchemaColumn {
                            name: c.name.clone(),
                            sql_type: sql_type(c.col_type).to_string(),
                            doc: c.doc.clone(),
                        })
                        .collect(),
                    relations: e
                        .relations
                        .iter()
                        .map(|r| SchemaRelation {
                            to: r.to.clone(),
                            on: r.on.clone(),
                        })
                        .collect(),
                    available: existing.contains(&e.view),
                });
            }
        }
    }
    Ok(all)
}

fn sql_type(t: ColumnType) -> &'static str {
    match t {
        ColumnType::Int | ColumnType::Bool => "INTEGER",
        ColumnType::Real => "REAL",
        ColumnType::Text | ColumnType::Time => "TEXT",
    }
}

/// Rows in one entity right now.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EntityRowCount {
    pub name: String,
    /// `None` for a declared entity that hasn't synced (no view yet).
    pub rows: Option<i64>,
}

/// Row counts for every entity in [`describe_schema`] (Settings → Data).
pub async fn row_counts(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
) -> Result<Vec<EntityRowCount>, DomainError> {
    let mut out = Vec::new();
    for e in describe_schema(layer, catalog, root).await? {
        let rows = if e.available {
            // Names come from the catalog (core views and validated
            // `v_<ext>_<entity>` names), never from user input.
            let r = layer
                .query_sql(&format!("SELECT count(*) FROM {}", e.name), vec![], Some(1))
                .await?;
            match r.rows.first().and_then(|row| row.first()) {
                Some(SqlCell::Int(n)) => Some(*n),
                _ => None,
            }
        } else {
            None
        };
        out.push(EntityRowCount { name: e.name, rows });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, EntityTable, SqliteExtSourceStore, StoredType};

    #[tokio::test]
    async fn row_counts_cover_every_entity_and_skip_unsynced_ones() {
        let root = tempfile::tempdir().unwrap();
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        )
        .unwrap();
        let db = Database::in_memory();
        let layer = crate::sql_gateway::SqlGateway::new(db.clone());
        let counts = row_counts(
            &layer,
            &crate::extension_catalog::ExtensionCatalog::new(),
            root.path(),
        )
        .await
        .unwrap();
        let get = |n: &str| counts.iter().find(|c| c.name == n).map(|c| c.rows);
        assert_eq!(get("v_task"), Some(Some(0)));
        assert_eq!(get("v_my_gh_pr"), Some(None), "not synced: no count");
        SqliteExtSourceStore::new(db)
            .replace_rows(vec![(
                EntityTable {
                    extension: "my-gh".into(),
                    entity: "pr".into(),
                    view: "v_my_gh_pr".into(),
                    key: "number".into(),
                    columns: vec![("number".into(), StoredType::Integer)],
                },
                vec![vec![SqlCell::Int(1)], vec![SqlCell::Int(2)]],
            )])
            .await
            .unwrap();
        let counts = row_counts(
            &layer,
            &crate::extension_catalog::ExtensionCatalog::new(),
            root.path(),
        )
        .await
        .unwrap();
        assert_eq!(
            counts.iter().find(|c| c.name == "v_my_gh_pr").unwrap().rows,
            Some(2)
        );
    }

    #[tokio::test]
    async fn includes_declared_extension_entities_and_tracks_availability() {
        let root = tempfile::tempdir().unwrap();
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - name: pr\n        doc: A pull request.\n        key: number\n        columns: { number: int, title: { type: text, doc: PR title } }\n        relations: [ { to: v_task, on: \"v_my_gh_pr.title LIKE '%' || v_task.id || '%'\" } ]\n",
        )
        .unwrap();
        let db = Database::in_memory();
        let layer = crate::sql_gateway::SqlGateway::new(db.clone());

        let all = describe_schema(
            &layer,
            &crate::extension_catalog::ExtensionCatalog::new(),
            root.path(),
        )
        .await
        .unwrap();
        assert!(all
            .iter()
            .any(|e| e.name == "v_task" && e.owner == "core" && e.available));
        let pr = all
            .iter()
            .find(|e| e.name == "v_my_gh_pr")
            .expect("extension entity listed");
        assert_eq!(pr.owner, "my-gh");
        assert_eq!(pr.description, "A pull request.");
        assert!(!pr.available, "not synced yet");
        assert_eq!(pr.columns[1].name, "title");
        assert_eq!(pr.columns[1].doc, "PR title");
        assert_eq!(pr.columns[0].sql_type, "INTEGER");
        assert_eq!(pr.relations[0].to, "v_task");

        SqliteExtSourceStore::new(db)
            .replace_rows(vec![(
                EntityTable {
                    extension: "my-gh".into(),
                    entity: "pr".into(),
                    view: "v_my_gh_pr".into(),
                    key: "number".into(),
                    columns: vec![
                        ("number".into(), StoredType::Integer),
                        ("title".into(), StoredType::Text),
                    ],
                },
                vec![],
            )])
            .await
            .unwrap();
        let all = describe_schema(
            &layer,
            &crate::extension_catalog::ExtensionCatalog::new(),
            root.path(),
        )
        .await
        .unwrap();
        assert!(
            all.iter()
                .find(|e| e.name == "v_my_gh_pr")
                .unwrap()
                .available
        );
    }
}
