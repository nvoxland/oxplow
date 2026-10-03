//! Which tables a write touched (P4.6, `.context/semantic-layer.md`
//! "Subscriptions"). Every pooled connection gets three hooks when it opens:
//!
//! - a **preupdate** hook notes each table a row change touches — unlike the
//!   plain update hook it fires for WITHOUT ROWID tables, and while it is set
//!   SQLite skips the truncate shortcut, so a bare `DELETE FROM t` reports
//!   too — and whether the change only inserted or **rewrote** (an UPDATE or
//!   a DELETE, P8.B3): appending to an incremental model is only right when
//!   none of its inputs was rewritten;
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

/// What one or more commits touched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// Every table a row changed in.
    pub tables: BTreeSet<String>,
    /// The ones it updated or deleted rows in, not only inserted into.
    pub rewrote: BTreeSet<String>,
}

impl Changed {
    /// Only inserts, into `tables`.
    pub fn inserted<I: IntoIterator<Item = String>>(tables: I) -> Self {
        Self {
            tables: tables.into_iter().collect(),
            rewrote: BTreeSet::new(),
        }
    }

    pub fn contains(&self, table: &str) -> bool {
        self.tables.contains(table)
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    fn note(&mut self, table: &str, rewrote: bool) {
        self.tables.insert(table.to_string());
        if rewrote {
            self.rewrote.insert(table.to_string());
        }
    }

    fn extend(&mut self, other: Changed) {
        self.tables.extend(other.tables);
        self.rewrote.extend(other.rewrote);
    }
}

/// What one or more commits touched, as published.
pub type TablesChanged = Arc<Changed>;

#[derive(Default)]
struct State {
    /// Per connection: what its open transaction touched.
    pending: HashMap<u64, Changed>,
    /// Touched by a commit, not yet published.
    committed: Changed,
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
            move |action: rusqlite::hooks::Action,
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
                        .note(table, action != rusqlite::hooks::Action::SQLITE_INSERT);
                }
            },
        ))?;
        let state = self.state.clone();
        conn.commit_hook(Some(move || {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(changed) = s.pending.remove(&id) {
                s.committed.extend(changed);
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
        let changed = std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .committed,
        );
        if !changed.is_empty() {
            // No subscriber is fine: nothing is waiting to hear.
            let _ = self.tx.send(Arc::new(changed));
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
            out.extend(t.tables.iter().cloned());
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

    /// P8.B3: a commit also says which tables it rewrote — updated or
    /// deleted rows in, not only inserted — so an incremental model knows
    /// when appending isn't enough.
    #[tokio::test]
    async fn commits_say_which_tables_they_rewrote() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE a (x INTEGER); CREATE TABLE b (x INTEGER);")
                .map_err(sql)
        })
        .await
        .unwrap();
        let mut rx = db.subscribe_changes();
        let next = |rx: &mut tokio::sync::broadcast::Receiver<super::TablesChanged>| {
            rx.try_recv().expect("a commit was published")
        };
        db.transaction(|tx| {
            tx.execute_batch("INSERT INTO a VALUES (1); INSERT INTO b VALUES (1);")
                .map_err(sql)
        })
        .await
        .unwrap();
        let only_inserts = next(&mut rx);
        assert_eq!(
            only_inserts.tables.iter().cloned().collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(only_inserts.rewrote.is_empty(), "{only_inserts:?}");

        db.transaction(|tx| {
            tx.execute_batch("UPDATE a SET x = 2; INSERT INTO b VALUES (2);")
                .map_err(sql)
        })
        .await
        .unwrap();
        let updated = next(&mut rx);
        assert_eq!(
            updated.rewrote.iter().cloned().collect::<Vec<_>>(),
            vec!["a".to_string()]
        );
        assert!(updated.contains("b"));

        db.call(|c| c.execute("DELETE FROM b", [])).await.unwrap();
        assert!(next(&mut rx).rewrote.contains("b"));
    }
}
