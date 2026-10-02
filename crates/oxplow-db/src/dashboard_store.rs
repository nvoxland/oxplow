//! User-created dashboards (epic tsk138): a `dashboard` plus its ordered
//! `dashboard_item` tiles. Project-global (no stream scope). A thin typed
//! read/write surface modeled on [`crate::agent_nudge_store`]; reordering
//! rewrites the whole tile list to dense `0..N` sort indices in one
//! transaction (the task-reorder pattern). See migration `V70__dashboard.sql`.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DashboardId, DashboardItemId, DomainError, Timestamp};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};

/// One dashboard (a named grid of tiles). Project-global.
///
/// `settings_json` is the saved default **view** — the filter row's range,
/// branch, and dimension filter — restored when the dashboard is next opened.
/// Opaque JSON for the same reason `DashboardItem::options_json` is: the filter
/// row will grow, and a blob grows without a migration. `None` = no saved view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Dashboard {
    pub id: DashboardId,
    pub title: String,
    pub sort_index: i64,
    pub settings_json: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One tile on a dashboard: its `kind` ([`TILE_KINDS`]) and `options_json`,
/// the per-tile options — a `query` tile's SQL and how it's displayed, a
/// `lens` tile's lens id, a `text` tile's text, and the size and filter
/// overrides every tile has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DashboardItem {
    pub id: DashboardItemId,
    pub dashboard_id: DashboardId,
    pub sort_index: i64,
    pub kind: String,
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

const DASH_COLS: &str = "id, title, sort_index, settings_json, created_at, updated_at";
const ITEM_COLS: &str = "id, dashboard_id, sort_index, kind, options_json, created_at, updated_at";

fn row_to_dashboard(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dashboard> {
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Dashboard {
        id: DashboardId::new(row.get(0)?),
        title: row.get(1)?,
        sort_index: row.get(2)?,
        settings_json: row.get(3)?,
        created_at: string_to_ts(&row.get::<_, String>(4)?).map_err(map_err)?,
        updated_at: string_to_ts(&row.get::<_, String>(5)?).map_err(map_err)?,
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
        options_json: row.get(4)?,
        created_at: string_to_ts(&row.get::<_, String>(5)?).map_err(map_err)?,
        updated_at: string_to_ts(&row.get::<_, String>(6)?).map_err(map_err)?,
    })
}

/// The tile kinds: `query` (pinned SQL, shown as a lens viz or the metric
/// card), `lens` (a lens by id) and `text` (a heading).
pub const TILE_KINDS: &[&str] = &["query", "lens", "text"];

/// New-tile input; `kind` is one of [`TILE_KINDS`].
#[derive(Debug, Clone)]
pub struct NewDashboardItem {
    pub kind: String,
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
}

fn now() -> String {
    ts_to_string(Timestamp::now())
}

/// A new, empty dashboard at the end of the list, on `conn` — a command's
/// transaction (every write is a `dashboard.*` command, P8.A5).
pub fn create_tx(conn: &Connection, title: &str) -> Result<DashboardId, DomainError> {
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
        params![title, next, now()],
    )
    .map_err(map_sql_err)?;
    Ok(DashboardId::new(conn.last_insert_rowid()))
}

/// Dashboard `id`, without its tiles.
pub fn dashboard_tx(conn: &Connection, id: DashboardId) -> Result<Option<Dashboard>, DomainError> {
    let sql = format!("SELECT {DASH_COLS} FROM dashboard WHERE id = ?1");
    let mut stmt = conn.prepare(&sql).map_err(map_sql_err)?;
    let mut rows = stmt
        .query_map(params![id.value()], row_to_dashboard)
        .map_err(map_sql_err)?;
    rows.next().transpose().map_err(map_sql_err)
}

/// A dashboard's tiles in their order.
pub fn items_tx(conn: &Connection, id: DashboardId) -> Result<Vec<DashboardItem>, DomainError> {
    let sql = format!(
        "SELECT {ITEM_COLS} FROM dashboard_item
          WHERE dashboard_id = ?1 ORDER BY sort_index ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![id.value()], row_to_item)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Tile `id`.
pub fn item_tx(
    conn: &Connection,
    id: DashboardItemId,
) -> Result<Option<DashboardItem>, DomainError> {
    let sql = format!("SELECT {ITEM_COLS} FROM dashboard_item WHERE id = ?1");
    let mut stmt = conn.prepare(&sql).map_err(map_sql_err)?;
    let mut rows = stmt
        .query_map(params![id.value()], row_to_item)
        .map_err(map_sql_err)?;
    rows.next().transpose().map_err(map_sql_err)
}

pub fn rename_tx(conn: &Connection, id: DashboardId, title: &str) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE dashboard SET title = ?2, updated_at = ?3 WHERE id = ?1",
        params![id.value(), title, now()],
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// Delete a dashboard; its tiles go with it (FK cascade).
pub fn delete_tx(conn: &Connection, id: DashboardId) -> Result<(), DomainError> {
    conn.execute("DELETE FROM dashboard WHERE id = ?1", params![id.value()])
        .map(|_| ())
        .map_err(map_sql_err)
}

/// Add a tile — at `position` (later tiles move down one), or at the end.
pub fn add_item_tx(
    conn: &Connection,
    dashboard: DashboardId,
    item: &NewDashboardItem,
    position: Option<i64>,
) -> Result<DashboardItemId, DomainError> {
    if !TILE_KINDS.contains(&item.kind.as_str()) {
        return Err(DomainError::Invalid(format!(
            "unknown tile kind `{}` ({})",
            item.kind,
            TILE_KINDS.join(" | ")
        )));
    }
    let at = match position {
        Some(at) => {
            conn.execute(
                "UPDATE dashboard_item SET sort_index = sort_index + 1
                  WHERE dashboard_id = ?1 AND sort_index >= ?2",
                params![dashboard.value(), at],
            )
            .map_err(map_sql_err)?;
            at
        }
        None => conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM dashboard_item WHERE dashboard_id = ?1",
                params![dashboard.value()],
                |r| r.get(0),
            )
            .map_err(map_sql_err)?,
    };
    conn.execute(
        "INSERT INTO dashboard_item
           (dashboard_id, sort_index, kind, options_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![dashboard.value(), at, item.kind, item.options_json, now()],
    )
    .map_err(map_sql_err)?;
    Ok(DashboardItemId::new(conn.last_insert_rowid()))
}

pub fn update_item_tx(
    conn: &Connection,
    id: DashboardItemId,
    options_json: Option<&str>,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE dashboard_item SET options_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id.value(), options_json, now()],
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

pub fn remove_item_tx(conn: &Connection, id: DashboardItemId) -> Result<(), DomainError> {
    conn.execute(
        "DELETE FROM dashboard_item WHERE id = ?1",
        params![id.value()],
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// The named tiles take the positions they're listed in.
pub fn reorder_items_tx(
    conn: &Connection,
    dashboard: DashboardId,
    order: &[DashboardItemId],
) -> Result<(), DomainError> {
    let now = now();
    for (idx, id) in order.iter().enumerate() {
        conn.execute(
            "UPDATE dashboard_item SET sort_index = ?2, updated_at = ?3
              WHERE id = ?1 AND dashboard_id = ?4",
            params![id.value(), idx as i64, now, dashboard.value()],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteDashboardStore {
        SqliteDashboardStore::new(Database::in_memory())
    }

    /// Run a `_tx` core in its own transaction.
    async fn tx<T: Send + 'static>(
        s: &SqliteDashboardStore,
        f: impl Fn(&Connection) -> Result<T, DomainError> + Send + 'static,
    ) -> Result<T, DomainError> {
        s.db.transaction(move |c| f(c)).await
    }

    fn query_tile(sql: &str) -> NewDashboardItem {
        NewDashboardItem {
            kind: "query".into(),
            options_json: Some(serde_json::json!({ "sql": sql, "display": "table" }).to_string()),
        }
    }

    #[tokio::test]
    async fn an_unknown_tile_kind_is_refused() {
        let s = store();
        let d = tx(&s, |c| create_tx(c, "D")).await.unwrap();
        let err = tx(&s, move |c| {
            create_tx(c, "x").and(add_item_tx(
                c,
                d,
                &NewDashboardItem {
                    kind: "metric".into(),
                    options_json: None,
                },
                None,
            ))
        })
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("unknown tile kind `metric`"),
            "{err}"
        );
    }

    /// P4.7 (tsk492): V108 turns a metric tile into a query tile reading
    /// its metric's captures, displayed as the metric card, keeping its
    /// other options.
    #[test]
    fn v108_turns_metric_tiles_into_query_tiles() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::migrate_to_for_tests(&mut conn, 107);
        conn.execute_batch(
            "INSERT INTO dashboard (id, title, sort_index, created_at, updated_at) VALUES (1, 'D', 0, 't', 't');
             INSERT INTO dashboard_item (dashboard_id, sort_index, kind, metric_key, options_json, created_at, updated_at)
               VALUES (1, 0, 'metric', 'it''s.a', '{\"viz\":\"number\",\"size\":\"wide\"}', 't', 't'),
                      (1, 1, 'text', NULL, '{\"text\":\"Hi\"}', 't', 't');",
        )
        .unwrap();
        crate::database::migrate_and_compile(&mut conn).unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT kind, options_json FROM dashboard_item ORDER BY sort_index")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows[0].0, "query");
        let opts: serde_json::Value = serde_json::from_str(&rows[0].1).unwrap();
        assert_eq!(opts["display"], "metric");
        assert_eq!(opts["metric"], "it's.a");
        assert_eq!(opts["viz"], "number");
        assert_eq!(opts["size"], "wide");
        let sql = opts["sql"].as_str().unwrap();
        assert!(sql.contains("MEASURE('it''s.a') AS value"), "{sql}");
        assert!(
            sql.contains("FROM metric_grid('capture') g LEFT JOIN v_capture c"),
            "{sql}"
        );
        assert_eq!(rows[1].0, "text");
        let cols: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('dashboard_item') WHERE name = 'metric_key'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 0);
    }

    /// tsk542: V119 names the version columns for what they hold
    /// (`closest_vcs_rev`, `vcs_rev_exact`) and rewrites the tiles V108
    /// pinned with the old names.
    #[test]
    fn v119_renames_the_vcs_rev_columns_and_the_pinned_tiles() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::migrate_to_for_tests(&mut conn, 107);
        conn.execute_batch(
            "INSERT INTO dashboard (id, title, sort_index, created_at, updated_at) VALUES (1, 'D', 0, 't', 't');
             INSERT INTO dashboard_item (dashboard_id, sort_index, kind, metric_key, options_json, created_at, updated_at)
               VALUES (1, 0, 'metric', 'm', '{}', 't', 't');",
        )
        .unwrap();
        crate::database::migrate_to_for_tests(&mut conn, 118);
        let sql = |conn: &rusqlite::Connection| -> String {
            conn.query_row(
                "SELECT json_extract(options_json, '$.sql') FROM dashboard_item",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(sql(&conn).contains("c.closest_git_version AS git_version"));
        crate::database::migrate_and_compile(&mut conn).unwrap();
        let after = sql(&conn);
        assert!(after.contains("c.closest_vcs_rev AS vcs_rev"), "{after}");
        assert!(!after.contains("git_version"), "{after}");
        for table in ["metric_capture", "page_ref", "effort_file"] {
            let names: Vec<String> = conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                names.contains(&"closest_vcs_rev".into())
                    && names.contains(&"vcs_rev_exact".into()),
                "{table}: {names:?}"
            );
            assert!(
                !names.iter().any(|n| n.contains("git")),
                "{table}: {names:?}"
            );
        }
        conn.prepare("SELECT closest_vcs_rev FROM v_capture")
            .unwrap();
    }

    #[tokio::test]
    async fn create_list_get_round_trip() {
        let s = store();
        let a = tx(&s, |c| create_tx(c, "Coverage")).await.unwrap();
        let b = tx(&s, |c| create_tx(c, "Complexity")).await.unwrap();
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
        let a = tx(&s, |c| create_tx(c, "Old")).await.unwrap();
        tx(&s, move |c| rename_tx(c, a, "New")).await.unwrap();
        assert_eq!(s.get(a).await.unwrap().unwrap().dashboard.title, "New");
        tx(&s, move |c| delete_tx(c, a)).await.unwrap();
        assert!(s.get(a).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn items_append_and_cascade_on_delete() {
        let s = store();
        let d = tx(&s, |c| create_tx(c, "D")).await.unwrap();
        let t1 = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 1"), None)
        })
        .await
        .unwrap();
        let t2 = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 2"), None)
        })
        .await
        .unwrap();
        let got = s.get(d).await.unwrap().unwrap();
        assert_eq!(got.items.len(), 2);
        assert_eq!(got.items[0].id, t1);
        assert_eq!(got.items[0].sort_index, 0);
        assert_eq!(got.items[1].id, t2);
        assert_eq!(got.items[1].sort_index, 1);
        assert_eq!(got.items[0].kind, "query");

        // Deleting the dashboard cascades to its tiles.
        tx(&s, move |c| delete_tx(c, d)).await.unwrap();
        assert!(s.get(d).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_and_remove_item() {
        let s = store();
        let d = tx(&s, |c| create_tx(c, "D")).await.unwrap();
        let t = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 1"), None)
        })
        .await
        .unwrap();
        tx(&s, move |c| {
            update_item_tx(c, t, Some(r#"{"viz":"number"}"#))
        })
        .await
        .unwrap();
        let got = s.get(d).await.unwrap().unwrap();
        assert_eq!(
            got.items[0].options_json.as_deref(),
            Some(r#"{"viz":"number"}"#)
        );
        tx(&s, move |c| remove_item_tx(c, t)).await.unwrap();
        assert!(s.get(d).await.unwrap().unwrap().items.is_empty());
    }

    #[tokio::test]
    async fn reorder_items_rewrites_sort_index() {
        let s = store();
        let d = tx(&s, |c| create_tx(c, "D")).await.unwrap();
        let t1 = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 1"), None)
        })
        .await
        .unwrap();
        let t2 = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 2"), None)
        })
        .await
        .unwrap();
        let t3 = tx(&s, move |c| {
            add_item_tx(c, d, &query_tile("SELECT 3"), None)
        })
        .await
        .unwrap();
        // Move t3 to the front.
        tx(&s, move |c| reorder_items_tx(c, d, &[t3, t1, t2]))
            .await
            .unwrap();
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
