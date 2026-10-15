//! `lens` and `extension_listing` (V74): the lenses and extensions the
//! catalog holds, restated whole by the app with each extension generation
//! and read as `v_lens` and `v_extension`.

use oxplow_domain::DomainError;
use rusqlite::params;

use crate::database::map_sql_err;

/// One lens, as `v_lens` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct LensRow {
    /// `<extension>/<slug>`.
    pub id: String,
    pub extension: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub viz: String,
    pub params_json: String,
    pub hidden: bool,
    pub launcher_category: Option<String>,
    pub needs_json: String,
    pub path: String,
}

/// One extension, as `v_extension` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionRow {
    pub name: String,
    pub namespace: String,
    pub description: String,
    pub path: String,
    pub origin: String,
    pub version: Option<String>,
    pub enabled: bool,
    pub sharing: String,
    pub purpose: Option<String>,
    pub source_git: Option<String>,
    pub source_sha: Option<String>,
    pub errors_json: String,
    pub warnings_json: String,
}

/// Replace every lens and extension row.
pub fn reset_tx(
    conn: &rusqlite::Connection,
    lenses: &[LensRow],
    extensions: &[ExtensionRow],
) -> Result<(), DomainError> {
    conn.execute("DELETE FROM lens", []).map_err(map_sql_err)?;
    conn.execute("DELETE FROM extension_listing", [])
        .map_err(map_sql_err)?;
    let mut lens = conn
        .prepare(
            "INSERT OR REPLACE INTO lens
               (id, extension, slug, title, description, viz, params_json, hidden,
                launcher_category, needs_json, path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(map_sql_err)?;
    for l in lenses {
        lens.execute(params![
            l.id,
            l.extension,
            l.slug,
            l.title,
            l.description,
            l.viz,
            l.params_json,
            l.hidden,
            l.launcher_category,
            l.needs_json,
            l.path,
        ])
        .map_err(map_sql_err)?;
    }
    let mut ext = conn
        .prepare(
            "INSERT OR REPLACE INTO extension_listing
               (name, namespace, description, path, origin, version, enabled, sharing,
                purpose, source_git, source_sha, errors_json, warnings_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )
        .map_err(map_sql_err)?;
    for e in extensions {
        ext.execute(params![
            e.name,
            e.namespace,
            e.description,
            e.path,
            e.origin,
            e.version,
            e.enabled,
            e.sharing,
            e.purpose,
            e.source_git,
            e.source_sha,
            e.errors_json,
            e.warnings_json,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}
