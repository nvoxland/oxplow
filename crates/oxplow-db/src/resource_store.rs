//! `resource` (V98): every resource the running vocabulary knows, as the
//! extensions declare them, restated whole by the app's
//! `vocabulary_reactor` on each rebuild; read as `v_resource`.

use rusqlite::{params, Connection};

use crate::database::map_sql_err;
use oxplow_domain::uri::resources::ResourceDecl;
use oxplow_domain::DomainError;

/// Replace the table with `rows`: each declaration and the extension
/// that declares it.
pub fn restate_tx(conn: &Connection, rows: &[(&ResourceDecl, &str)]) -> Result<(), DomainError> {
    conn.execute("DELETE FROM resource", [])
        .map_err(map_sql_err)?;
    let mut st = conn
        .prepare(
            "INSERT INTO resource
                 (resource, extension, label, params, additional, resolve, title, searchable, icon,
                  wikilink, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(map_sql_err)?;
    for (d, extension) in rows {
        st.execute(params![
            d.name.to_string(),
            extension,
            d.label,
            serde_json::to_string(&d.params).expect("params serialize"),
            d.additional,
            d.resolve,
            d.title,
            d.searchable,
            d.icon,
            d.wikilink
                .as_ref()
                .map(|w| serde_json::to_string(w).expect("a wikilink serializes")),
            d.description,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}
