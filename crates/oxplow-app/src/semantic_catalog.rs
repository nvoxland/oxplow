//! Settings → Data: what the semantic layer holds, who provides it, and how
//! much. The catalog itself is the model registry (`v_model`,
//! `v_model_column`, P4.2/P4.9) — read through SQL like everything else;
//! this adds what SQL can't say: an entity an extension declares that
//! hasn't synced yet. Row counts aren't here: one `count(*)` over a big
//! model can outrun the query timeout, so the UI counts each row itself
//! (tsk1065). See `.context/semantic-layer.md`.

use std::collections::BTreeSet;
use std::path::Path;

use oxplow_db::SqlCell;
use oxplow_domain::DomainError;

/// One row of Settings → Data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DataEntity {
    /// The view to query (`v_work_item`, `v_my_gh_pr`).
    pub name: String,
    /// `core`, or the extension that provides it.
    pub owner: String,
    /// `sql` (a model file), `entity` (an extension's synced data), or
    /// `declared` (an entity whose source hasn't synced: no view yet).
    pub kind: String,
    pub description: String,
}

/// Every published model, then every entity the
/// enabled extensions under `root` declare but haven't synced.
pub async fn data_entities(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
) -> Result<Vec<DataEntity>, DomainError> {
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => t.clone(),
        _ => String::new(),
    };
    let models = layer
        .query_sql(
            "SELECT view, owner, kind, description FROM v_model
              ORDER BY owner <> 'core', owner, view",
            vec![],
            Some(oxplow_db::semantic_layer::MAX_ROW_LIMIT),
        )
        .await?;
    let mut out: Vec<DataEntity> = models
        .rows
        .iter()
        .map(|row| DataEntity {
            name: text(&row[0]),
            owner: text(&row[1]),
            kind: text(&row[2]),
            description: text(&row[3]),
        })
        .collect();
    let published: BTreeSet<String> = out.iter().map(|e| e.name.clone()).collect();
    for ext in catalog.get(root).iter().filter(|e| e.enabled) {
        for collector in &ext.collectors {
            for e in &collector.entities {
                if published.contains(&e.view) {
                    continue;
                }
                out.push(DataEntity {
                    name: e.view.clone(),
                    owner: ext.name.clone(),
                    kind: "declared".into(),
                    description: if e.doc.is_empty() {
                        format!(
                            "`{}` records from the `{}` collector of extension `{}`.",
                            e.name, collector.id, ext.name
                        )
                    } else {
                        e.doc.clone()
                    },
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, EntityColumn, EntityTable, SqliteCollectorStore, StoredType};

    /// tsk517: Settings → Data lists every published model
    /// — a synced entity once, from the registry, with its doc — and a
    /// declared entity that hasn't synced as `declared`.
    #[tokio::test]
    async fn data_entities_cover_models_and_unsynced_entities() {
        let root = tempfile::tempdir().unwrap();
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: my-gh\nintent:\n  purpose: test\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, doc: A pull request., key: number, columns: { number: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("sync.sh"), "#!/bin/sh\n").unwrap();
        let db = Database::in_memory();
        let layer = crate::sql_gateway::SqlGateway::new(db.clone());
        let catalog = crate::extension_catalog::ExtensionCatalog::new();
        let all = data_entities(&layer, &catalog, root.path()).await.unwrap();
        let get = |all: &[DataEntity], n: &str| {
            all.iter()
                .filter(|e| e.name == n)
                .cloned()
                .collect::<Vec<_>>()
        };
        let task = get(&all, "v_work_item");
        assert_eq!(
            (task[0].owner.as_str(), task[0].kind.as_str()),
            ("core", "sql")
        );
        let pr = get(&all, "v_my_gh_pr");
        assert_eq!(pr.len(), 1);
        assert_eq!(pr[0].kind, "declared");
        assert_eq!(pr[0].description, "A pull request.");

        SqliteCollectorStore::new(db)
            .replace_rows(vec![(
                EntityTable {
                    extension: "my-gh".into(),
                    entity: "pr".into(),
                    view: "v_my_gh_pr".into(),
                    key: "number".into(),
                    description: "A pull request.".into(),
                    columns: vec![EntityColumn {
                        name: "number".into(),
                        stored: StoredType::Integer,
                        doc: "PR number.".into(),
                    }],
                },
                vec![vec![SqlCell::Int(1)], vec![SqlCell::Int(2)]],
            )])
            .await
            .unwrap();
        let all = data_entities(&layer, &catalog, root.path()).await.unwrap();
        let pr = get(&all, "v_my_gh_pr");
        assert_eq!(pr.len(), 1, "listed once, from the registry");
        assert_eq!(
            (
                pr[0].owner.as_str(),
                pr[0].kind.as_str(),
                pr[0].description.as_str()
            ),
            ("my-gh", "entity", "A pull request.")
        );
    }
}
