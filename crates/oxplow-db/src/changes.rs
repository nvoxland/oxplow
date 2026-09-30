//! Which tables a write touched (P4.6, `.context/semantic-layer.md`
//! "Subscriptions"). Every pooled connection gets three hooks when it opens:
//!
//! - a **preupdate** hook notes each table a row change touches — unlike the
//!   plain update hook it fires for WITHOUT ROWID tables, and while it is set
//!   SQLite skips the truncate shortcut, so a bare `DELETE FROM t` reports
//!   too;
//! - a **commit** hook moves the connection's noted tables to the committed
//!   set;
//! - a **rollback** hook forgets them.
//!
//! After each `Database` call the committed set is published on a broadcast
//! channel — after the commit, so a subscriber that reads on hearing it sees
//! the change. Temp tables (a query's own grid) aren't reported.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tokio::sync::broadcast;

/// The tables one or more commits touched.
pub type TablesChanged = Arc<BTreeSet<String>>;

#[derive(Default)]
struct State {
    /// Per connection: tables touched by its open transaction.
    pending: HashMap<u64, BTreeSet<String>>,
    /// Touched by a commit, not yet published.
    committed: BTreeSet<String>,
}

pub struct Changes {
    state: Arc<Mutex<State>>,
    next_conn: AtomicU64,
    tx: broadcast::Sender<TablesChanged>,
}

impl Default for Changes {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            next_conn: AtomicU64::new(0),
            tx: broadcast::channel(256).0,
        }
    }
}

impl Changes {
    /// Install the hooks on a newly opened connection.
    pub(crate) fn install(&self, conn: &Connection) -> rusqlite::Result<()> {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let state = self.state.clone();
        conn.preupdate_hook(Some(
            move |_: rusqlite::hooks::Action,
                  db: &str,
                  table: &str,
                  _: &rusqlite::hooks::PreUpdateCase| {
                if db == "main" {
                    state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pending
                        .entry(id)
                        .or_default()
                        .insert(table.to_string());
                }
            },
        ))?;
        let state = self.state.clone();
        conn.commit_hook(Some(move || {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(tables) = s.pending.remove(&id) {
                s.committed.extend(tables);
            }
            false // don't turn the commit into a rollback
        }))?;
        let state = self.state.clone();
        conn.rollback_hook(Some(move || {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&id);
        }))?;
        Ok(())
    }

    /// Publish what has been committed since the last publish.
    pub(crate) fn flush(&self) {
        let tables = std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .committed,
        );
        if !tables.is_empty() {
            // No subscriber is fine: nothing is waiting to hear.
            let _ = self.tx.send(Arc::new(tables));
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TablesChanged> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;
    use oxplow_domain::DomainError;

    fn tables(rx: &mut tokio::sync::broadcast::Receiver<super::TablesChanged>) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        while let Ok(t) = rx.try_recv() {
            out.extend(t.iter().cloned());
        }
        out.into_iter().collect()
    }

    fn sql(e: rusqlite::Error) -> DomainError {
        crate::database::map_sql_err(e)
    }

    /// P4.6 (tsk491): a commit says which tables it touched — a bare
    /// `DELETE FROM` (SQLite's truncate shortcut) and a WITHOUT ROWID table
    /// too; a rollback, a temp table and a read say nothing.
    #[tokio::test]
    async fn commits_report_the_tables_they_touched() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "CREATE TABLE w (k TEXT PRIMARY KEY, v INTEGER) WITHOUT ROWID;
                 CREATE TABLE plain (x INTEGER);
                 INSERT INTO plain VALUES (1), (2);",
            )
            .map_err(sql)
        })
        .await
        .unwrap();
        let mut rx = db.subscribe_changes();
        db.transaction(|tx| {
            tx.execute("INSERT INTO w VALUES ('a', 1)", [])
                .map_err(sql)?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(tables(&mut rx), vec!["w".to_string()]);
        db.call(|c| c.execute("DELETE FROM plain", []))
            .await
            .unwrap();
        assert_eq!(tables(&mut rx), vec!["plain".to_string()]);
        // Rolled back: nothing.
        let _ = db
            .transaction(|tx| {
                tx.execute("INSERT INTO w VALUES ('b', 2)", [])
                    .map_err(sql)?;
                Err::<(), _>(DomainError::Invalid("no".into()))
            })
            .await;
        assert!(tables(&mut rx).is_empty());
        // A temp table and a read: nothing.
        db.call(|c| c.execute_batch("CREATE TEMP TABLE t (x); INSERT INTO t VALUES (1)"))
            .await
            .unwrap();
        db.read(|tx| {
            tx.query_row("SELECT count(*) FROM w", [], |r| r.get::<_, i64>(0))
                .map_err(sql)
        })
        .await
        .unwrap();
        assert!(tables(&mut rx).is_empty());
    }
}
