//! `extension_listing` (V74), `page`, `page_answer` and `page_param`
//! (V110) and `config_key` (V85): the pages and extensions the catalog
//! holds, and what can be configured, restated whole by the app with each
//! extension generation and read as `v_page`, `v_extension` and
//! `v_config_key`. What a person can ask is
//! what pages answer (`v_page_answer`).

use oxplow_domain::DomainError;
use rusqlite::params;

use crate::database::map_sql_err;

/// One page file, as `v_page` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRow {
    pub uri: String,
    pub extension: String,
    /// `<extension>/<slug>`.
    pub id: String,
    pub title: String,
    pub description: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    pub keywords: Option<String>,
    pub params_json: String,
    pub renders_json: String,
    pub is_default: bool,
    pub pinned: bool,
    pub hidden: bool,
    pub tool: Option<String>,
    pub needs_json: String,
    /// Its source file, the one to edit; none when no source is here.
    pub source: Option<String>,
    pub deployment: String,
    /// The file URI of the copy that runs, pinned to its deployment; none
    /// with no deployment.
    pub running_uri: Option<String>,
}

/// One question a page answers, as `v_page_answer` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PageAnswerRow {
    pub page_uri: String,
    /// From 1, in the page's order.
    pub position: i64,
    pub question: String,
    pub about: Option<String>,
    pub params_json: String,
}

/// One param a page takes, as `v_page_param` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PageParamRow {
    pub page_uri: String,
    /// From 1, in the page's order.
    pub position: i64,
    pub name: String,
    pub r#type: String,
    pub role: String,
    pub label: Option<String>,
    pub default_json: Option<String>,
    pub options_json: String,
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
    /// What an undeployed extension waits on a person for, as a JSON array.
    pub pending_json: String,
    /// It is the team's.
    pub shared: bool,
}

/// One project.yaml key, as `v_config_key` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigKeyRow {
    pub key: String,
    pub doc: String,
    pub schema_json: String,
}

/// What the catalog restates with each generation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogListing {
    pub pages: Vec<PageRow>,
    pub page_answers: Vec<PageAnswerRow>,
    pub page_params: Vec<PageParamRow>,
    pub extensions: Vec<ExtensionRow>,
    /// In catalog order: each row's position is its index, from 1.
    pub config_keys: Vec<ConfigKeyRow>,
}

/// The extensions with no deployment running: listed for their source
/// (`undeployed`) or for the setting that names them (`not_here`).
const SOURCE_BACKED: &str = "deployment IN ('undeployed', 'not_here')";

/// Restate the rows that follow a source on disk, and no others: those
/// are the only rows a source change can move, so a restate between
/// generations leaves what runs (and every other table) as it was.
pub fn restate_source_backed_tx(
    conn: &rusqlite::Connection,
    extensions: &[ExtensionRow],
) -> Result<(), DomainError> {
    conn.execute(
        &format!("DELETE FROM extension_listing WHERE {SOURCE_BACKED}"),
        [],
    )
    .map_err(map_sql_err)?;
    let backed: Vec<ExtensionRow> = extensions
        .iter()
        .filter(|e| matches!(e.deployment.as_str(), "undeployed" | "not_here"))
        .cloned()
        .collect();
    insert_extensions(conn, &backed)
}

fn insert_extensions(
    conn: &rusqlite::Connection,
    extensions: &[ExtensionRow],
) -> Result<(), DomainError> {
    let mut ext = conn
        .prepare(
            "INSERT OR REPLACE INTO extension_listing
               (name, namespace, description, path, origin, version, enabled,
                purpose, deployment, hash, errors_json, warnings_json, permissions_json,
                pending_json, shared)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            e.pending_json,
            e.shared,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}

fn insert_pages(conn: &rusqlite::Connection, listing: &CatalogListing) -> Result<(), DomainError> {
    for table in ["page", "page_answer", "page_param"] {
        conn.execute(&format!("DELETE FROM {table}"), [])
            .map_err(map_sql_err)?;
    }
    let mut page = conn
        .prepare(
            "INSERT OR REPLACE INTO page
               (uri, extension, id, title, description, label, icon, category, keywords,
                params_json, renders_json, is_default, pinned, hidden, tool, needs_json,
                source, deployment, running_uri)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        )
        .map_err(map_sql_err)?;
    for p in &listing.pages {
        page.execute(params![
            p.uri,
            p.extension,
            p.id,
            p.title,
            p.description,
            p.label,
            p.icon,
            p.category,
            p.keywords,
            p.params_json,
            p.renders_json,
            p.is_default,
            p.pinned,
            p.hidden,
            p.tool,
            p.needs_json,
            p.source,
            p.deployment,
            p.running_uri,
        ])
        .map_err(map_sql_err)?;
    }
    let mut answer = conn
        .prepare(
            "INSERT OR REPLACE INTO page_answer (page_uri, position, question, about, params_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(map_sql_err)?;
    for a in &listing.page_answers {
        answer
            .execute(params![
                a.page_uri,
                a.position,
                a.question,
                a.about,
                a.params_json
            ])
            .map_err(map_sql_err)?;
    }
    let mut param = conn
        .prepare(
            "INSERT OR REPLACE INTO page_param
               (page_uri, position, name, type, role, label, default_json, options_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(map_sql_err)?;
    for p in &listing.page_params {
        param
            .execute(params![
                p.page_uri,
                p.position,
                p.name,
                p.r#type,
                p.role,
                p.label,
                p.default_json,
                p.options_json
            ])
            .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Replace every page, extension and config key row.
pub fn reset_tx(conn: &rusqlite::Connection, listing: &CatalogListing) -> Result<(), DomainError> {
    let extensions = &listing.extensions;
    conn.execute("DELETE FROM extension_listing", [])
        .map_err(map_sql_err)?;
    insert_pages(conn, listing)?;
    insert_extensions(conn, extensions)?;
    conn.execute("DELETE FROM config_key", [])
        .map_err(map_sql_err)?;
    let mut key = conn
        .prepare(
            "INSERT OR REPLACE INTO config_key (key, doc, schema_json)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(map_sql_err)?;
    for k in &listing.config_keys {
        key.execute(params![k.key, k.doc, k.schema_json])
            .map_err(map_sql_err)?;
    }
    Ok(())
}
