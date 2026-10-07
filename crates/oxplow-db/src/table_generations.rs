//! Per-table change generations (`.context/semantic-layer.md` "Assets"):
//! a counter per table, bumped by triggers inside the very transaction
//! that writes a row — whatever path wrote it, a store core in a
//! transaction or an autocommit statement. What an asset was built from
//! is then durable: after a restart, an asset whose input generations
//! haven't moved needn't rebuild.
//!
//! Only tables something asks to track carry the triggers ([`track_tx`]),
//! so a table no asset reads pays nothing. A table without them is
//! untracked, and reads as `None`: its changes can't be known.

use std::collections::BTreeMap;

use oxplow_domain::DomainError;
use rusqlite::{params, Connection, OptionalExtension};

use crate::database::map_sql_err;

/// The triggers' names for `table`.
fn trigger(table: &str, op: &str) -> String {
    format!("table_generation_{op}_{table}")
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Give each of `tables` that exists the triggers that bump its
/// generation on every inserted, updated or deleted row. Idempotent.
pub fn track_tx(conn: &Connection, tables: &[String]) -> Result<(), DomainError> {
    for table in tables {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(map_sql_err)?;
        if !exists || table == "table_generation" {
            continue;
        }
        let name = table.replace('\'', "''");
        for (op, event) in [("i", "INSERT"), ("u", "UPDATE"), ("d", "DELETE")] {
            conn.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS {trigger} AFTER {event} ON {table}
                 BEGIN
                   INSERT INTO table_generation (name, gen) VALUES ('{name}', 1)
                   ON CONFLICT (name) DO UPDATE SET gen = gen + 1;
                 END;",
                trigger = quoted(&trigger(table, op)),
                table = quoted(table),
            ))
            .map_err(map_sql_err)?;
        }
    }
    Ok(())
}

/// Each of `tables`' generation: how many rows were written to it since it
/// was first tracked (0 for none), or `None` when it isn't tracked — its
/// changes can't be known.
pub fn generations_tx(
    conn: &Connection,
    tables: &[String],
) -> Result<BTreeMap<String, Option<i64>>, DomainError> {
    let mut out = BTreeMap::new();
    for table in tables {
        let tracked: bool = conn
            .query_row(
                "SELECT count(*) = 3 FROM sqlite_master WHERE type = 'trigger' AND name IN (?1, ?2, ?3)",
                params![
                    trigger(table, "i"),
                    trigger(table, "u"),
                    trigger(table, "d")
                ],
                |r| r.get(0),
            )
            .map_err(map_sql_err)?;
        let gen = if tracked {
            Some(
                conn.query_row(
                    "SELECT gen FROM table_generation WHERE name = ?1",
                    [table],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(map_sql_err)?
                .unwrap_or(0),
            )
        } else {
            None
        };
        out.insert(table.clone(), gen);
    }
    Ok(out)
}

/// This build of the program: the running executable's path, size and
/// modification time. Derived data a different build computed may differ
/// from what this one would, so it doesn't count as current.
pub fn build_identity() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(exe) = std::env::current_exe() {
        exe.hash(&mut h);
        if let Ok(meta) = std::fs::metadata(&exe) {
            meta.len().hash(&mut h);
            if let Ok(modified) = meta.modified() {
                modified.hash(&mut h);
            }
        }
    }
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// A tracked table's generation moves with every written row, through a
    /// transaction or an autocommit statement; an untracked one is unknown.
    #[tokio::test]
    async fn generations_follow_every_write_path() {
        let db = Database::in_memory();
        let tables = vec!["scratch".to_string(), "other".to_string()];
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE scratch (x); CREATE TABLE other (x);")
                .map_err(map_sql_err)?;
            track_tx(tx, &["scratch".to_string()])
        })
        .await
        .unwrap();
        let gens = || {
            let tables = tables.clone();
            db.read(move |tx| generations_tx(tx, &tables))
        };
        assert_eq!(gens().await.unwrap()["scratch"], Some(0));
        assert_eq!(gens().await.unwrap()["other"], None, "untracked");
        db.transaction(|tx| {
            tx.execute("INSERT INTO scratch VALUES (1), (2)", [])
                .map_err(map_sql_err)
        })
        .await
        .unwrap();
        assert_eq!(gens().await.unwrap()["scratch"], Some(2));
        db.call(|c| c.execute("UPDATE scratch SET x = 3 WHERE x = 1", []))
            .await
            .unwrap();
        db.call(|c| c.execute("DELETE FROM scratch", []))
            .await
            .unwrap();
        assert_eq!(gens().await.unwrap()["scratch"], Some(5));
        // Tracking again changes nothing.
        db.transaction(|tx| track_tx(tx, &["scratch".to_string()]))
            .await
            .unwrap();
        assert_eq!(gens().await.unwrap()["scratch"], Some(5));
    }
}
