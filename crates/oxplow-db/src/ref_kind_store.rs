//! `ref_kind` (V146, P8.D6): every kind of ref the running vocabulary
//! knows, restated whole by the app's `vocabulary_reactor` on each
//! rebuild; read as `v_ref_kind`.

use rusqlite::{params, Connection};

use crate::database::map_sql_err;
use oxplow_domain::DomainError;

/// One kind, as `v_ref_kind` lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RefKindRow {
    pub kind: String,
    /// `None` for a core kind.
    pub extension: Option<String>,
    pub label: Option<String>,
    pub id_pattern: String,
    pub revisioned: bool,
    pub wikilinks: Vec<String>,
    pub resolve: Option<String>,
    pub page: Option<String>,
    pub icon: Option<String>,
    /// The view search indexes under the kind; `None` for a core kind
    /// (core indexes its own) and an extension's kind that isn't searchable.
    pub searchable: Option<String>,
}

/// Replace the table with `rows`.
pub fn restate_tx(conn: &Connection, rows: &[RefKindRow]) -> Result<(), DomainError> {
    conn.execute("DELETE FROM ref_kind", [])
        .map_err(map_sql_err)?;
    let mut st = conn
        .prepare(
            "INSERT INTO ref_kind
                 (kind, extension, label, id_pattern, revisioned, wikilinks, resolve, page, icon,
                  searchable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(map_sql_err)?;
    for r in rows {
        st.execute(params![
            r.kind,
            r.extension,
            r.label,
            r.id_pattern,
            r.revisioned,
            serde_json::to_string(&r.wikilinks).unwrap_or_else(|_| "[]".into()),
            r.resolve,
            r.page,
            r.icon,
            r.searchable,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}
