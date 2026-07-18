//! User-created dashboards (epic tsk138): a `dashboard` plus its ordered
//! `dashboard_item` tiles. Project-global (no stream scope). A thin typed
//! read/write surface modeled on [`crate::agent_nudge_store`]; reordering
//! rewrites the whole tile list to dense `0..N` sort indices in one
//! transaction (the task-reorder pattern). See migration `V70__dashboard.sql`.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DashboardId, DashboardItemId, DomainError, Timestamp};

use crate::database::{map_sql_err, Database};

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

/// One dashboard (a named grid of tiles). Project-global.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Dashboard {
    pub id: DashboardId,
    pub title: String,
    pub sort_index: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One tile on a dashboard. `kind` is `metric` | `text`; `metric_key` names the
/// charted metric (null for text tiles); `options_json` is the opaque per-tile
/// options blob (viz/mode/scale/size/overrides/text body).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DashboardItem {
    pub id: DashboardItemId,
    pub dashboard_id: DashboardId,
    pub sort_index: i64,
    pub kind: String,
    pub metric_key: Option<String>,
    pub options_json: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A dashboard plus its tiles, in display order — the `get_dashboard` read shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DashboardWithItems {
    pub dashboard: Dashboard,
    pub items: Vec<DashboardItem>,
}

const DASH_COLS: &str = "id, title, sort_index, created_at, updated_at";
const ITEM_COLS: &str =
    "id, dashboard_id, sort_index, kind, metric_key, options_json, created_at, updated_at";

fn row_to_dashboard(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dashboard> {
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Dashboard {
        id: DashboardId::new(row.get(0)?),
        title: row.get(1)?,
        sort_index: row.get(2)?,
        created_at: string_to_ts(&row.get::<_, String>(3)?).map_err(map_err)?,
        updated_at: string_to_ts(&row.get::<_, String>(4)?).map_err(map_err)?,
    })
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<DashboardItem> {
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(DashboardItem {
        id: DashboardItemId::new(row.get(0)?),
        dashboard_id: DashboardId::new(row.get(1)?),
        sort_index: row.get(2)?,
        kind: row.get(3)?,
        metric_key: row.get(4)?,
        options_json: row.get(5)?,
        created_at: string_to_ts(&row.get::<_, String>(6)?).map_err(map_err)?,
        updated_at: string_to_ts(&row.get::<_, String>(7)?).map_err(map_err)?,
    })
}

/// New-tile input; `kind` is `metric` | `text`.
#[derive(Debug, Clone)]
pub struct NewDashboardItem {
    pub kind: String,
    pub metric_key: Option<String>,
    pub options_json: Option<String>,
}

#[derive(Clone)]
pub struct SqliteDashboardStore {
    db: Database,
}

impl SqliteDashboardStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Every dashboard, in display order.
    pub async fn list(&self) -> Result<Vec<Dashboard>, DomainError> {
        self.db
            .call(move |conn| {
                let sql =
                    format!("SELECT {DASH_COLS} FROM dashboard ORDER BY sort_index ASC, id ASC");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_dashboard)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// A dashboard plus its ordered tiles, or `None` if it doesn't exist.
    pub async fn get(&self, id: DashboardId) -> Result<Option<DashboardWithItems>, DomainError> {
        let id_val = id.value();
        self.db
            .call(move |conn| {
                let dash = {
                    let sql = format!("SELECT {DASH_COLS} FROM dashboard WHERE id = ?1");
                    let mut stmt = conn.prepare(&sql)?;
                    let mut rows = stmt.query_map(params![id_val], row_to_dashboard)?;
                    match rows.next() {
                        Some(r) => r?,
                        None => return Ok(None),
                    }
                };
                let items = {
                    let sql = format!(
                        "SELECT {ITEM_COLS} FROM dashboard_item
                          WHERE dashboard_id = ?1 ORDER BY sort_index ASC, id ASC"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt.query_map(params![id_val], row_to_item)?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(Some(DashboardWithItems {
                    dashboard: dash,
                    items,
                }))
            })
            .await
    }

    /// Create an empty dashboard appended at the end. Returns its id.
    pub async fn create(&self, title: String) -> Result<DashboardId, DomainError> {
        self.db
            .call_mut(move |conn| {
                let now = ts_to_string(Timestamp::now());
                let next: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM dashboard",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(map_sql_err)?;
                conn.execute(
                    "INSERT INTO dashboard (title, sort_index, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![title, next, now],
                )
                .map_err(map_sql_err)?;
                Ok(DashboardId::new(conn.last_insert_rowid()))
            })
            .await
    }

    /// Rename a dashboard. No-op if it doesn't exist.
    pub async fn rename(&self, id: DashboardId, title: String) -> Result<(), DomainError> {
        let id_val = id.value();
        self.db
            .call_mut(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "UPDATE dashboard SET title = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id_val, title, now],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Delete a dashboard and (via ON DELETE CASCADE) its tiles.
    pub async fn delete(&self, id: DashboardId) -> Result<(), DomainError> {
        let id_val = id.value();
        self.db
            .call_mut(move |conn| {
                conn.execute("DELETE FROM dashboard WHERE id = ?1", params![id_val])
                    .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Add a tile to a dashboard, appended at the end. Returns its id.
    pub async fn add_item(
        &self,
        dashboard_id: DashboardId,
        item: NewDashboardItem,
    ) -> Result<DashboardItemId, DomainError> {
        let dash_val = dashboard_id.value();
        self.db
            .call_mut(move |conn| {
                let now = ts_to_string(Timestamp::now());
                let next: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM dashboard_item WHERE dashboard_id = ?1",
                        params![dash_val],
                        |r| r.get(0),
                    )
                    .map_err(map_sql_err)?;
                conn.execute(
                    "INSERT INTO dashboard_item
                       (dashboard_id, sort_index, kind, metric_key, options_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    params![dash_val, next, item.kind, item.metric_key, item.options_json, now],
                )
                .map_err(map_sql_err)?;
                Ok(DashboardItemId::new(conn.last_insert_rowid()))
            })
            .await
    }

    /// Update a tile's metric_key / options. No-op if it doesn't exist.
    pub async fn update_item(
        &self,
        id: DashboardItemId,
        metric_key: Option<String>,
        options_json: Option<String>,
    ) -> Result<(), DomainError> {
        let id_val = id.value();
        self.db
            .call_mut(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "UPDATE dashboard_item
                        SET metric_key = ?2, options_json = ?3, updated_at = ?4
                      WHERE id = ?1",
                    params![id_val, metric_key, options_json, now],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Remove a tile.
    pub async fn remove_item(&self, id: DashboardItemId) -> Result<(), DomainError> {
        let id_val = id.value();
        self.db
            .call_mut(move |conn| {
                conn.execute("DELETE FROM dashboard_item WHERE id = ?1", params![id_val])
                    .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Reorder a dashboard's tiles: rewrite `sort_index` to the position of each
    /// id in `order` (dense `0..N`), in one transaction. Ids not belonging to
    /// the dashboard are skipped (scope guard).
    pub async fn reorder_items(
        &self,
        dashboard_id: DashboardId,
        order: Vec<DashboardItemId>,
    ) -> Result<(), DomainError> {
        let dash_val = dashboard_id.value();
        self.db
            .call_mut(move |conn| {
                let now = ts_to_string(Timestamp::now());
                let tx = conn.transaction().map_err(map_sql_err)?;
                for (idx, id) in order.iter().enumerate() {
                    tx.execute(
                        "UPDATE dashboard_item SET sort_index = ?2, updated_at = ?3
                          WHERE id = ?1 AND dashboard_id = ?4",
                        params![id.value(), idx as i64, now, dash_val],
                    )
                    .map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteDashboardStore {
        SqliteDashboardStore::new(Database::in_memory())
    }

    fn metric_tile(key: &str) -> NewDashboardItem {
        NewDashboardItem {
            kind: "metric".into(),
            metric_key: Some(key.into()),
            options_json: Some(r#"{"viz":"line"}"#.into()),
        }
    }

    #[tokio::test]
    async fn create_list_get_round_trip() {
        let s = store();
        let a = s.create("Coverage".into()).await.unwrap();
        let b = s.create("Complexity".into()).await.unwrap();
        let list = s.list().await.unwrap();
        assert_eq!(list.len(), 2);
        // Ordered by sort_index (creation order): a then b.
        assert_eq!(list[0].id, a);
        assert_eq!(list[0].title, "Coverage");
        assert_eq!(list[1].id, b);

        let got = s.get(a).await.unwrap().expect("exists");
        assert_eq!(got.dashboard.id, a);
        assert!(got.items.is_empty());
        assert!(s.get(DashboardId::new(999)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rename_and_delete() {
        let s = store();
        let a = s.create("Old".into()).await.unwrap();
        s.rename(a, "New".into()).await.unwrap();
        assert_eq!(s.get(a).await.unwrap().unwrap().dashboard.title, "New");
        s.delete(a).await.unwrap();
        assert!(s.get(a).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn items_append_and_cascade_on_delete() {
        let s = store();
        let d = s.create("D".into()).await.unwrap();
        let t1 = s
            .add_item(d, metric_tile("oxplow.coverage.abs_pct"))
            .await
            .unwrap();
        let t2 = s
            .add_item(d, metric_tile("repo.complexity_avg"))
            .await
            .unwrap();
        let got = s.get(d).await.unwrap().unwrap();
        assert_eq!(got.items.len(), 2);
        assert_eq!(got.items[0].id, t1);
        assert_eq!(got.items[0].sort_index, 0);
        assert_eq!(got.items[1].id, t2);
        assert_eq!(got.items[1].sort_index, 1);
        assert_eq!(
            got.items[0].metric_key.as_deref(),
            Some("oxplow.coverage.abs_pct")
        );

        // Deleting the dashboard cascades to its tiles.
        s.delete(d).await.unwrap();
        assert!(s.get(d).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_and_remove_item() {
        let s = store();
        let d = s.create("D".into()).await.unwrap();
        let t = s.add_item(d, metric_tile("a")).await.unwrap();
        s.update_item(t, Some("b".into()), Some(r#"{"viz":"number"}"#.into()))
            .await
            .unwrap();
        let got = s.get(d).await.unwrap().unwrap();
        assert_eq!(got.items[0].metric_key.as_deref(), Some("b"));
        assert_eq!(
            got.items[0].options_json.as_deref(),
            Some(r#"{"viz":"number"}"#)
        );
        s.remove_item(t).await.unwrap();
        assert!(s.get(d).await.unwrap().unwrap().items.is_empty());
    }

    #[tokio::test]
    async fn reorder_items_rewrites_sort_index() {
        let s = store();
        let d = s.create("D".into()).await.unwrap();
        let t1 = s.add_item(d, metric_tile("a")).await.unwrap();
        let t2 = s.add_item(d, metric_tile("b")).await.unwrap();
        let t3 = s.add_item(d, metric_tile("c")).await.unwrap();
        // Move t3 to the front.
        s.reorder_items(d, vec![t3, t1, t2]).await.unwrap();
        let got = s.get(d).await.unwrap().unwrap();
        assert_eq!(
            got.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![t3, t1, t2]
        );
        assert_eq!(
            got.items.iter().map(|i| i.sort_index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
}
