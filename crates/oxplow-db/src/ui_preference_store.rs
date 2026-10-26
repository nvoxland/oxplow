//! What a person's oxplow remembers for them outside any one tab
//! (`ui_preference`): a page's **page memory** (scoped by the page's own
//! ref, whatever its params — `oxplow:oxplow/page?page=tasks`, `oxplow:oxplow/page?page=acme/pulls`,
//! `oxplow:oxplow/lens?lens=acme/board` — or `kind:<kind>` for every page of an entity kind),
//! a panel's (its `panel:` ref), a custom component's own
//! (`component:<ext>/<id>`), and the app's **app preferences** (`app`).
//! A tab's own view and history state live in its tab session, not here.
//! Local to this project's database, never the repo, like `panel_layout`.

use oxplow_domain::{DomainError, Timestamp};
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;

/// The most one value may be, in bytes of JSON.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
/// The most one component's memory may be, all its keys together.
pub const MAX_COMPONENT_BYTES: usize = 1024 * 1024;

/// One remembered value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiPreference {
    /// `app`; a page's URI (`oxplow:oxplow/page?…`)
    /// or `resource:<extension>/<resource>`; a panel's URI
    /// (`oxplow:oxplow/panel?…`); or `component:<ext>/<id>`.
    pub scope: String,
    pub key: String,
    /// Its value, as JSON text.
    pub value_json: String,
}

/// Why `scope` isn't one a preference may have. A page's or panel's scope
/// is its URI; `resource:` and `component:` scopes aren't URIs.
fn scope_problem(scope: &str) -> Option<String> {
    use oxplow_domain::uri::resources::{PAGE, PANEL};
    use oxplow_domain::uri::OxplowUri;
    let a_uri = OxplowUri::parse(scope)
        .is_ok_and(|u| [PAGE, PANEL].contains(&u.resource().to_string().as_str()));
    let named = |prefix: &str| {
        scope
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty())
    };
    if scope == "app" || a_uri || named("resource:") || named("component:") {
        None
    } else {
        Some(format!(
            "`{scope}` isn't a preference scope: `app`, a page's URI (`oxplow:oxplow/page?…`) or `resource:<extension>/<resource>`, a panel's URI (`oxplow:oxplow/panel?…`), or `component:<extension>/<id>`"
        ))
    }
}

pub struct SqliteUiPreferenceStore {
    db: Database,
}

impl SqliteUiPreferenceStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Every remembered value, by scope then key.
    pub async fn all(&self) -> Result<Vec<UiPreference>, DomainError> {
        self.db
            .read(|c| {
                let mut stmt = c
                    .prepare("SELECT scope, key, value_json FROM ui_preference ORDER BY scope, key")
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(UiPreference {
                            scope: r.get(0)?,
                            key: r.get(1)?,
                            value_json: r.get(2)?,
                        })
                    })
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
    }

    /// Remember `value_json` under `scope` and `key`; `None` forgets it.
    ///
    /// # Errors
    /// `Invalid` for an unknown scope, an empty key, text that isn't JSON,
    /// a value over [`MAX_VALUE_BYTES`], or a component's memory that would
    /// pass [`MAX_COMPONENT_BYTES`].
    pub async fn set(
        &self,
        scope: String,
        key: String,
        value_json: Option<String>,
    ) -> Result<(), DomainError> {
        if let Some(problem) = scope_problem(&scope) {
            return Err(DomainError::Invalid(problem));
        }
        if key.is_empty() {
            return Err(DomainError::Invalid("a preference needs a key".into()));
        }
        if let Some(value) = &value_json {
            serde_json::from_str::<serde_json::Value>(value)
                .map_err(|e| DomainError::Invalid(format!("`{key}`'s value isn't JSON: {e}")))?;
            if value.len() > MAX_VALUE_BYTES {
                return Err(DomainError::Invalid(format!(
                    "`{key}`'s value is {} bytes; a value is {MAX_VALUE_BYTES} at most",
                    value.len()
                )));
            }
        }
        let now = Timestamp::now().to_string();
        self.db
            .transaction(move |tx| {
                let Some(value) = value_json.as_deref() else {
                    tx.execute(
                        "DELETE FROM ui_preference WHERE scope = ?1 AND key = ?2",
                        rusqlite::params![scope, key],
                    )
                    .map_err(map_sql_err)?;
                    return Ok(());
                };
                if scope.starts_with("component:") {
                    let others: i64 = tx
                        .query_row(
                            "SELECT COALESCE(SUM(length(value_json)), 0) FROM ui_preference WHERE scope = ?1 AND key <> ?2",
                            rusqlite::params![scope, key],
                            |r| r.get(0),
                        )
                        .map_err(map_sql_err)?;
                    let total = others as usize + value.len();
                    if total > MAX_COMPONENT_BYTES {
                        return Err(DomainError::Invalid(format!(
                            "`{scope}` would keep {total} bytes; a component keeps {MAX_COMPONENT_BYTES} at most"
                        )));
                    }
                }
                tx.execute(
                    "INSERT INTO ui_preference (scope, key, value_json, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (scope, key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
                    rusqlite::params![scope, key, value, now],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pref(scope: &str, key: &str, value: &str) -> UiPreference {
        UiPreference {
            scope: scope.into(),
            key: key.into(),
            value_json: value.into(),
        }
    }

    /// A value is remembered per scope and key, replaced by a new one, and
    /// forgotten by none.
    #[tokio::test]
    async fn a_value_is_kept_replaced_and_forgotten() {
        let store = SqliteUiPreferenceStore::new(Database::in_memory());
        assert!(store.all().await.unwrap().is_empty());
        store
            .set("app".into(), "railWidth".into(), Some("320".into()))
            .await
            .unwrap();
        store
            .set(
                "oxplow:oxplow/page?page=git-dashboard".into(),
                "mergeMode".into(),
                Some("\"rebase\"".into()),
            )
            .await
            .unwrap();
        store
            .set("app".into(), "railWidth".into(), Some("400".into()))
            .await
            .unwrap();
        assert_eq!(
            store.all().await.unwrap(),
            vec![
                pref("app", "railWidth", "400"),
                pref(
                    "oxplow:oxplow/page?page=git-dashboard",
                    "mergeMode",
                    "\"rebase\""
                )
            ]
        );
        store
            .set("app".into(), "railWidth".into(), None)
            .await
            .unwrap();
        assert_eq!(
            store.all().await.unwrap(),
            vec![pref(
                "oxplow:oxplow/page?page=git-dashboard",
                "mergeMode",
                "\"rebase\""
            )]
        );
    }

    /// Only the four scopes, a key, and JSON within the size caps.
    #[tokio::test]
    async fn what_isnt_a_preference_is_refused() {
        let store = SqliteUiPreferenceStore::new(Database::in_memory());
        let refused = |r: Result<(), DomainError>| matches!(r, Err(DomainError::Invalid(_)));
        assert!(refused(
            store.set("tab".into(), "k".into(), Some("1".into())).await
        ));
        assert!(refused(
            store
                .set("page:".into(), "k".into(), Some("1".into()))
                .await
        ));
        for scope in [
            "oxplow:oxplow/lens?lens=acme/board",
            "resource:oxplow/file",
            "oxplow:oxplow/panel?panel=core:work",
        ] {
            store
                .set(scope.into(), "k".into(), Some("1".into()))
                .await
                .unwrap();
        }
        assert!(refused(
            store.set("app".into(), "".into(), Some("1".into())).await
        ));
        assert!(refused(
            store
                .set("app".into(), "k".into(), Some("not json".into()))
                .await
        ));
        let big = format!("\"{}\"", "x".repeat(MAX_VALUE_BYTES));
        assert!(refused(
            store.set("app".into(), "k".into(), Some(big)).await
        ));
        // A component's keys together stay under its cap.
        let chunk = format!("\"{}\"", "x".repeat(MAX_VALUE_BYTES - 10));
        for i in 0..(MAX_COMPONENT_BYTES / MAX_VALUE_BYTES) {
            store
                .set(
                    "component:acme/board".into(),
                    format!("k{i}"),
                    Some(chunk.clone()),
                )
                .await
                .unwrap();
        }
        assert!(refused(
            store
                .set(
                    "component:acme/board".into(),
                    "more".into(),
                    Some(chunk.clone())
                )
                .await
        ));
        // Replacing a key counts its new size, not both.
        store
            .set("component:acme/board".into(), "k0".into(), Some(chunk))
            .await
            .unwrap();
    }
}
