//! `lens` and `extension_listing` (V74), `catalog_prompt` and `config_key`
//! (V85): the lenses and extensions the catalog holds, what a person can
//! ask and what they can configure, restated whole by the app with each
//! extension generation and read as `v_lens`, `v_extension`, `v_prompt`
//! and `v_config_key`.

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
    pub purpose: Option<String>,
    /// `deployed`, `shipped`, `source`, `undeployed` or `not_here`.
    pub deployment: String,
    pub hash: Option<String>,
    pub errors_json: String,
    pub warnings_json: String,
    /// What it declares it may reach beyond the baseline, as JSON.
    pub permissions_json: String,
}

/// One question a person can ask, as `v_prompt` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptRow {
    /// `area` or `extension`.
    pub source_kind: String,
    pub source: String,
    pub prompt: String,
    pub about: Option<String>,
}

/// One project.yaml key, as `v_config_key` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigKeyRow {
    pub key: String,
    pub doc: String,
    pub human_only: bool,
    pub schema_json: String,
}

/// What the catalog restates with each generation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogListing {
    pub lenses: Vec<LensRow>,
    pub extensions: Vec<ExtensionRow>,
    /// In catalog order: each row's position is its index, from 1.
    pub prompts: Vec<PromptRow>,
    pub config_keys: Vec<ConfigKeyRow>,
}

/// Replace every lens, extension, prompt and config key row.
pub fn reset_tx(conn: &rusqlite::Connection, listing: &CatalogListing) -> Result<(), DomainError> {
    let (lenses, extensions) = (&listing.lenses, &listing.extensions);
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
               (name, namespace, description, path, origin, version, enabled,
                purpose, deployment, hash, errors_json, warnings_json, permissions_json)
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
            e.purpose,
            e.deployment,
            e.hash,
            e.errors_json,
            e.warnings_json,
            e.permissions_json,
        ])
        .map_err(map_sql_err)?;
    }
    conn.execute("DELETE FROM catalog_prompt", [])
        .map_err(map_sql_err)?;
    let mut prompt = conn
        .prepare(
            "INSERT INTO catalog_prompt (position, source_kind, source, prompt, about)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(map_sql_err)?;
    for (i, p) in listing.prompts.iter().enumerate() {
        prompt
            .execute(params![
                i as i64 + 1,
                p.source_kind,
                p.source,
                p.prompt,
                p.about
            ])
            .map_err(map_sql_err)?;
    }
    conn.execute("DELETE FROM config_key", [])
        .map_err(map_sql_err)?;
    let mut key = conn
        .prepare(
            "INSERT OR REPLACE INTO config_key (key, doc, human_only, schema_json)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(map_sql_err)?;
    for k in &listing.config_keys {
        key.execute(params![k.key, k.doc, k.human_only, k.schema_json])
            .map_err(map_sql_err)?;
    }
    Ok(())
}
