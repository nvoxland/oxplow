//! `extension_deployment` (V88): each deployment of an extension — an
//! immutable copy of its source in the store, named by the source's hash —
//! and which one each extension runs now (`active`). The app records a row
//! when it copies a deployment, marks the active ones in the transaction
//! that publishes an extension generation, and forgets rows it prunes. Read
//! as `v_extension_deployment` (`.context/extensions.md` "Deployments").

use oxplow_domain::DomainError;
use rusqlite::params;

use crate::database::map_sql_err;

/// One deployment.
#[derive(Debug, Clone, PartialEq)]
pub struct DeploymentRow {
    pub name: String,
    pub hash: String,
    pub deployed_at: String,
    pub from_stream: Option<i64>,
    /// The source folder it was copied from.
    pub source: String,
    pub active: bool,
}

/// Record a deployment (inactive). One already recorded keeps its row.
pub fn record_tx(conn: &rusqlite::Connection, row: &DeploymentRow) -> Result<(), DomainError> {
    conn.execute(
        "INSERT OR IGNORE INTO extension_deployment
           (name, hash, deployed_at, from_stream, source, active)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![
            row.name,
            row.hash,
            row.deployed_at,
            row.from_stream,
            row.source
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Mark exactly `live` (`(name, hash)`) active; every other row inactive.
pub fn set_active_tx(
    conn: &rusqlite::Connection,
    live: &[(String, String)],
) -> Result<(), DomainError> {
    conn.execute("UPDATE extension_deployment SET active = 0", [])
        .map_err(map_sql_err)?;
    let mut mark = conn
        .prepare("UPDATE extension_deployment SET active = 1 WHERE name = ?1 AND hash = ?2")
        .map_err(map_sql_err)?;
    for (name, hash) in live {
        mark.execute(params![name, hash]).map_err(map_sql_err)?;
    }
    Ok(())
}

/// Every deployment of `name`, newest first; every extension's when `name`
/// is `None`.
pub fn list_tx(
    conn: &rusqlite::Connection,
    name: Option<&str>,
) -> Result<Vec<DeploymentRow>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT name, hash, deployed_at, from_stream, source, active
             FROM extension_deployment
             WHERE ?1 IS NULL OR name = ?1
             ORDER BY name, deployed_at DESC, hash",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![name], |r| {
            Ok(DeploymentRow {
                name: r.get(0)?,
                hash: r.get(1)?,
                deployed_at: r.get(2)?,
                from_stream: r.get(3)?,
                source: r.get(4)?,
                active: r.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(map_sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_err)?;
    Ok(rows)
}

/// Forget `name`'s deployments `hashes`; all of them when `hashes` is `None`.
pub fn forget_tx(
    conn: &rusqlite::Connection,
    name: &str,
    hashes: Option<&[String]>,
) -> Result<(), DomainError> {
    match hashes {
        None => {
            conn.execute(
                "DELETE FROM extension_deployment WHERE name = ?1",
                params![name],
            )
            .map_err(map_sql_err)?;
        }
        Some(hashes) => {
            let mut stmt = conn
                .prepare("DELETE FROM extension_deployment WHERE name = ?1 AND hash = ?2")
                .map_err(map_sql_err)?;
            for hash in hashes {
                stmt.execute(params![name, hash]).map_err(map_sql_err)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn row(name: &str, hash: &str, at: &str) -> DeploymentRow {
        DeploymentRow {
            name: name.into(),
            hash: hash.into(),
            deployed_at: at.into(),
            from_stream: Some(1),
            source: format!("/src/{name}"),
            active: false,
        }
    }

    #[tokio::test]
    async fn deployments_are_recorded_marked_active_and_forgotten() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            record_tx(tx, &row("a", "h1", "2026-10-18T10:00:00Z"))?;
            record_tx(tx, &row("a", "h2", "2026-10-18T11:00:00Z"))?;
            record_tx(tx, &row("b", "h3", "2026-10-18T09:00:00Z"))?;
            // Recording one again keeps its first row.
            record_tx(tx, &row("a", "h1", "2026-10-18T12:00:00Z"))?;
            set_active_tx(tx, &[("a".into(), "h2".into()), ("b".into(), "h3".into())])
        })
        .await
        .unwrap();
        let a = db.read(|c| list_tx(c, Some("a"))).await.unwrap();
        assert_eq!(
            a.iter()
                .map(|r| (r.hash.as_str(), r.deployed_at.as_str(), r.active))
                .collect::<Vec<_>>(),
            [
                ("h2", "2026-10-18T11:00:00Z", true),
                ("h1", "2026-10-18T10:00:00Z", false)
            ]
        );

        db.transaction(|tx| {
            set_active_tx(tx, &[("a".into(), "h1".into())])?;
            forget_tx(tx, "a", Some(&["h2".to_string()]))?;
            forget_tx(tx, "b", None)
        })
        .await
        .unwrap();
        let all = db.read(|c| list_tx(c, None)).await.unwrap();
        assert_eq!(
            all.iter()
                .map(|r| (r.name.as_str(), r.hash.as_str(), r.active))
                .collect::<Vec<_>>(),
            [("a", "h1", true)]
        );
        let model: Vec<(String, Option<String>, i64)> = db
            .read(|c| {
                c.query_row(
                    "SELECT ref, extension_ref, active FROM v_extension_deployment",
                    [],
                    |r| Ok(vec![(r.get(0)?, r.get(1)?, r.get(2)?)]),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(
            model,
            // (`extension:a` once the catalog lists `a`; nothing lists it here.)
            [("extension_deployment:a@h1".into(), None, 1)]
        );
    }
}
