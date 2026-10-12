//! `contribution` (V47): every enabled extension's contributions, as their
//! descriptors say (`oxplow_domain::contribution`). Restated whole by the
//! app on each catalog change (`oxplow_app::capabilities::refresh`); read
//! as `v_contribution`.

use oxplow_domain::contribution::Descriptor;
use oxplow_domain::DomainError;
use rusqlite::params;

use crate::database::{map_sql_err, Database};

fn json(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).expect("a descriptor field serializes")
}

/// Replace every row with `descriptors`.
pub fn reset_tx(
    conn: &rusqlite::Connection,
    descriptors: &[Descriptor],
) -> Result<(), DomainError> {
    conn.execute("DELETE FROM contribution", [])
        .map_err(map_sql_err)?;
    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO contribution
               (id, extension, kind, local, origin, version, contract_version,
                runs_on_json, needs_json, after_json, global_name, aliases_json, file, line)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .map_err(map_sql_err)?;
    for d in descriptors {
        stmt.execute(params![
            d.id.to_string(),
            d.id.extension,
            d.id.kind.segment(),
            d.id.local,
            d.owner.origin,
            d.owner.version,
            d.contract_version,
            json(&d.trigger),
            json(&d.needs),
            json(&d.after),
            d.global,
            json(&d.aliases),
            d.source.file,
            d.source.line,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// [`reset_tx`] in its own transaction.
pub async fn reset(db: &Database, descriptors: Vec<Descriptor>) -> Result<(), DomainError> {
    db.transaction(move |tx| reset_tx(tx, &descriptors)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::contribution::{
        ContributionId, ContributionKind, DeclaredAt, Descriptor, Owner, RunsOn,
    };

    #[tokio::test]
    async fn a_reset_restates_every_row() {
        let db = Database::in_memory();
        let d = |ext: &str, local: &str| {
            let mut d = Descriptor::new(
                ContributionId::new(ext, ContributionKind::Metric, local),
                Owner {
                    extension: ext.into(),
                    origin: "project".into(),
                    version: Some("1.0.0".into()),
                },
                RunsOn::Query,
                DeclaredAt {
                    file: format!("oxplow/extensions/{ext}/extension.yaml"),
                    line: None,
                },
            );
            d.global = Some(local.into());
            d
        };
        reset(&db, vec![d("alpha", "acme.loc"), d("beta", "acme.todos")])
            .await
            .unwrap();
        reset(&db, vec![d("beta", "acme.todos")]).await.unwrap();
        let rows: Vec<(String, String, Option<String>)> = db
            .call(|c| {
                let mut s =
                    c.prepare("SELECT id, kind, version FROM v_contribution ORDER BY id")?;
                let rows = s
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            [(
                "beta/metric/acme.todos".to_string(),
                "metric".to_string(),
                Some("1.0.0".to_string())
            )]
        );
    }

    /// Every kind is stable, so neither the table nor its model carries a
    /// per-kind stability.
    #[tokio::test]
    async fn the_table_and_its_model_have_no_stability_column() {
        let db = Database::in_memory();
        let (table, view): (Vec<String>, Vec<String>) = db
            .call(|c| {
                let cols = |sql: &str| -> rusqlite::Result<Vec<String>> {
                    c.prepare(sql)?.query_map([], |r| r.get(1))?.collect()
                };
                Ok((
                    cols("PRAGMA table_info(contribution)")?,
                    cols("PRAGMA table_info(v_contribution)")?,
                ))
            })
            .await
            .unwrap();
        assert!(table.contains(&"contract_version".to_string()), "{table:?}");
        assert!(view.contains(&"contract_version".to_string()), "{view:?}");
        assert!(!table.iter().chain(&view).any(|c| c == "stability"));
    }
}
