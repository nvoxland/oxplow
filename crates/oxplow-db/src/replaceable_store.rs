//! `replaceable` (V57): the core sub-components an extension may replace
//! (`oxplow_domain::replaceable::all()`), restated whole by the
//! app's `vocabulary_reactor` on each rebuild; read as `v_replaceable`.

use rusqlite::{params, Connection};

use crate::database::map_sql_err;
use oxplow_domain::replaceable::all as replaceable_all;
use oxplow_domain::DomainError;

/// Replace the table with the one table there is.
pub fn restate_tx(conn: &Connection) -> Result<(), DomainError> {
    conn.execute("DELETE FROM replaceable", [])
        .map_err(map_sql_err)?;
    let mut st = conn
        .prepare(
            "INSERT INTO replaceable (target, capability, bound, version, props, view, label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(map_sql_err)?;
    for r in replaceable_all() {
        st.execute(params![
            r.target,
            r.capability,
            r.bound_to_provider as i64,
            i64::from(r.version),
            serde_json::to_string(r.props).expect("prop names serialize"),
            serde_json::to_string(r.view).expect("view names serialize"),
            r.label
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}
