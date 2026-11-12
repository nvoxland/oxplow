//! An extension's own SQLite file (`storage: { sqlite: true }`,
//! `.context/extensions-permissions.md` "Store services"): its own pool, never
//! attached to the main database, its migrations recorded inside it.

use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);

/// One statement and its positional parameters (`?1`, `?2`, …).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Value>,
}

/// What a call answers: for each statement, in order, the rows it returns
/// (objects, column to value; a `SELECT`'s, or an `INSERT`/`UPDATE`/`DELETE`
/// with `RETURNING`'s; empty for a statement that returns none), and the
/// rows changed by all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub rows: Vec<Vec<Value>>,
    pub changes: usize,
}

#[derive(Clone)]
pub struct ExtensionStore {
    pool: Pool<SqliteConnectionManager>,
}

fn fail(error: impl std::fmt::Display) -> StoreError {
    StoreError(error.to_string())
}

impl ExtensionStore {
    /// Open (create when missing) the file at `path` with a pool of its own.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        let setup = Connection::open(path).map_err(fail)?;
        setup
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(fail)?;
        drop(setup);
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.pragma_update(None, "foreign_keys", "ON")?;
            // Nothing may attach another file to this connection, whatever
            // the statement check misses: the file is the boundary.
            c.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0)?;
            c.busy_timeout(std::time::Duration::from_secs(5))
        });
        let pool = Pool::builder().max_size(2).build(manager).map_err(fail)?;
        Ok(Self { pool })
    }

    /// Run `(file name, sql)` migrations, each named `V<n>__<name>.sql`, in
    /// order of `n`; the ones already recorded in the file are skipped. A
    /// migration and its record are one transaction.
    pub fn migrate(&self, migrations: &[(String, String)]) -> Result<(), StoreError> {
        let mut ordered = Vec::new();
        for (file, sql) in migrations {
            ordered.push((version_of(file)?, file, sql));
        }
        ordered.sort_by_key(|(version, ..)| *version);
        let mut conn = self.pool.get().map_err(fail)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _oxplow_migration \
             (version INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        )
        .map_err(fail)?;
        for (version, file, sql) in ordered {
            let done: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM _oxplow_migration WHERE version = ?1)",
                    [version],
                    |r| r.get(0),
                )
                .map_err(fail)?;
            if done {
                continue;
            }
            let tx = conn.transaction().map_err(fail)?;
            guarded(&tx, |c| c.execute_batch(sql))?
                .map_err(|e| StoreError(format!("migration {file}: {e}")))?;
            tx.execute(
                "INSERT INTO _oxplow_migration (version, name) VALUES (?1, ?2)",
                rusqlite::params![version, file],
            )
            .map_err(fail)?;
            tx.commit().map_err(fail)?;
        }
        Ok(())
    }

    /// The statements in one transaction: all or none.
    pub fn write(&self, batch: &[Statement]) -> Result<Answer, StoreError> {
        let mut conn = self.pool.get().map_err(fail)?;
        let tx = conn.transaction().map_err(fail)?;
        let answer = guarded(&tx, |c| execute(c, batch))??;
        tx.commit().map_err(fail)?;
        Ok(answer)
    }

    /// One statement that changes nothing: a `SELECT` (or a read-form
    /// `PRAGMA`). Anything that writes is refused when it is prepared.
    pub fn read(&self, statement: &Statement) -> Result<Answer, StoreError> {
        use rusqlite::hooks::{AuthAction, Authorization};
        let conn = self.pool.get().map_err(fail)?;
        conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
            match ctx.action {
                AuthAction::Select
                | AuthAction::Read { .. }
                | AuthAction::Function { .. }
                | AuthAction::Recursive
                | AuthAction::Pragma {
                    pragma_value: None, ..
                } => Authorization::Allow,
                _ => Authorization::Deny,
            }
        }))
        .map_err(fail)?;
        let answer = execute(&conn, std::slice::from_ref(statement));
        let _ = conn.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>);
        answer
    }
}

/// Run `f` on the extension's own SQL under an authorizer that refuses
/// `ATTACH`, `DETACH` and transaction control (`BEGIN`, `COMMIT`, `ROLLBACK`,
/// `SAVEPOINT`, `RELEASE`): the host owns the transaction and the file is
/// the boundary. The authorizer is removed again before the host's own SQL.
fn guarded<T>(conn: &Connection, f: impl FnOnce(&Connection) -> T) -> Result<T, StoreError> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    conn.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Attach { .. }
        | AuthAction::Detach { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }))
    .map_err(fail)?;
    let out = f(conn);
    let _ = conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    Ok(out)
}

fn execute(tx: &Connection, batch: &[Statement]) -> Result<Answer, StoreError> {
    {
        let mut answer = Answer {
            rows: Vec::new(),
            changes: 0,
        };
        for statement in batch {
            refuse_escape(&statement.sql)?;
            let mut prepared = tx.prepare(&statement.sql).map_err(fail)?;
            let params: Vec<rusqlite::types::Value> = statement.params.iter().map(cell).collect();
            if prepared.column_count() == 0 {
                answer.changes += prepared
                    .execute(rusqlite::params_from_iter(params))
                    .map_err(fail)?;
                answer.rows.push(Vec::new());
                continue;
            }
            let names: Vec<String> = prepared
                .column_names()
                .into_iter()
                .map(str::to_string)
                .collect();
            let mut rows = Vec::new();
            let mut cursor = prepared
                .query(rusqlite::params_from_iter(params))
                .map_err(fail)?;
            while let Some(row) = cursor.next().map_err(fail)? {
                let mut object = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    object.insert(name.clone(), json(row.get_ref(i).map_err(fail)?));
                }
                rows.push(Value::Object(object));
            }
            drop(cursor);
            // A `SELECT` changes nothing (`changes()` would still hold the
            // statement before it); `RETURNING` of a write does.
            let wrote = !prepared.readonly();
            drop(prepared);
            if wrote {
                answer.changes += tx.changes() as usize;
            }
            answer.rows.push(rows);
        }
        Ok(answer)
    }
}

/// Whether `file` is named `V<n>__<name>.sql`.
pub fn check_migration_name(file: &str) -> Result<(), StoreError> {
    version_of(file).map(|_| ())
}

/// `V<n>__<name>.sql`'s `n`.
fn version_of(file: &str) -> Result<i64, StoreError> {
    file.strip_prefix('V')
        .and_then(|rest| rest.split_once("__"))
        .and_then(|(n, name)| (name.ends_with(".sql") && name.len() > 4).then_some(n))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| {
            StoreError(format!(
                "migration `{file}` isn't named V<n>__<name>.sql (e.g. V1__book.sql)"
            ))
        })
}

/// A file of its own is the boundary: a statement that names another file
/// (`ATTACH`, `VACUUM INTO`) is refused.
fn refuse_escape(sql: &str) -> Result<(), StoreError> {
    let mut rest = sql.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    let word = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    match word.as_str() {
        "attach" | "detach" | "vacuum" => Err(StoreError(format!(
            "`{}` isn't allowed: a store is its extension's own file",
            word.to_ascii_uppercase()
        ))),
        _ => Ok(()),
    }
}

fn cell(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Cell;
    match value {
        Value::Null => Cell::Null,
        Value::Bool(b) => Cell::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(Cell::Integer)
            .or_else(|| n.as_f64().map(Cell::Real))
            .unwrap_or(Cell::Null),
        Value::String(s) => Cell::Text(s.clone()),
        other => Cell::Text(other.to_string()),
    }
}

fn json(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Array(b.iter().map(|byte| Value::from(*byte)).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stmt(sql: &str, params: Vec<Value>) -> Statement {
        Statement {
            sql: sql.into(),
            params,
        }
    }

    fn migrations() -> Vec<(String, String)> {
        vec![
            (
                "V2__seed.sql".into(),
                "INSERT INTO book (id, title) VALUES ('b0', 'Seed');".into(),
            ),
            (
                "V1__book.sql".into(),
                "CREATE TABLE book (id TEXT PRIMARY KEY, title TEXT NOT NULL);".into(),
            ),
        ]
    }

    /// The rows of an answer to one statement.
    fn only(answer: Answer) -> Vec<Value> {
        assert_eq!(answer.rows.len(), 1, "one statement: {answer:?}");
        answer.rows.into_iter().next().unwrap()
    }

    /// A write answers each statement's own `RETURNING` rows (a list per
    /// statement, empty for one without), so the store can mint an id in its
    /// write; `changes` counts only the statements that changed rows.
    #[test]
    fn a_write_answers_each_statements_returning_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        store.migrate(&migrations()).unwrap();
        let answer = store
            .write(&[
                stmt(
                    "INSERT INTO book (id, title) VALUES ('m-' || (SELECT count(*) FROM book), ?1) RETURNING id, title",
                    vec![json!("Dune")],
                ),
                stmt("UPDATE book SET title = title WHERE id = 'b0'", vec![]),
                stmt("SELECT count(*) AS n FROM book", vec![]),
            ])
            .unwrap();
        assert_eq!(
            answer.rows,
            vec![
                vec![json!({ "id": "m-1", "title": "Dune" })],
                vec![],
                vec![json!({ "n": 2 })],
            ]
        );
        assert_eq!(answer.changes, 2, "a SELECT changes nothing");
    }

    #[test]
    fn migrations_run_in_order_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acme.sqlite");
        let store = ExtensionStore::open(&path).unwrap();
        store.migrate(&migrations()).unwrap();
        // A second open of the same file runs none of them again (V2 would
        // fail on the primary key).
        let store = ExtensionStore::open(&path).unwrap();
        store.migrate(&migrations()).unwrap();
        let answer = store
            .write(&[stmt("SELECT count(*) AS n FROM book", vec![])])
            .unwrap();
        assert_eq!(only(answer), vec![json!({ "n": 1 })]);
    }

    #[test]
    fn a_failed_migration_is_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        let bad = vec![("V1__bad.sql".to_string(), "CREATE TABLE t (".to_string())];
        assert!(store.migrate(&bad).is_err());
        let good = vec![("V1__t.sql".to_string(), "CREATE TABLE t (x);".to_string())];
        store.migrate(&good).unwrap();
    }

    #[test]
    fn a_batch_is_one_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        store.migrate(&migrations()).unwrap();
        let failed = store.write(&[
            stmt(
                "INSERT INTO book VALUES (?1, ?2)",
                vec![json!("b1"), json!("Dune")],
            ),
            stmt(
                "INSERT INTO book VALUES (?1, ?2)",
                vec![json!("b1"), json!("Dup")],
            ),
        ]);
        assert!(failed.is_err());
        let count = store
            .write(&[stmt("SELECT id FROM book ORDER BY id", vec![])])
            .unwrap();
        assert_eq!(only(count), vec![json!({ "id": "b0" })]);
        let ok = store
            .write(&[
                stmt(
                    "INSERT INTO book VALUES (?1, ?2)",
                    vec![json!("b1"), json!("Dune")],
                ),
                stmt(
                    "INSERT INTO book VALUES (?1, ?2)",
                    vec![json!("b2"), json!(null)],
                )
                .clone(),
            ])
            .unwrap_err();
        // The second statement breaks NOT NULL: the first is undone too.
        assert!(ok.0.contains("NOT NULL"), "{ok}");
        let n = store
            .write(&[stmt("SELECT count(*) AS n FROM book", vec![])])
            .unwrap();
        assert_eq!(only(n), vec![json!({ "n": 1 })]);
    }

    #[test]
    fn a_write_answers_its_changes_and_parameters_bind() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        store.migrate(&migrations()).unwrap();
        let answer = store
            .write(&[stmt(
                "UPDATE book SET title = ?1 WHERE id = ?2",
                vec![json!("New"), json!("b0")],
            )])
            .unwrap();
        assert_eq!((answer.rows, answer.changes), (vec![vec![]], 1));
    }

    #[test]
    fn a_read_refuses_anything_that_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        store.migrate(&migrations()).unwrap();
        let read = |sql: &str| store.read(&stmt(sql, vec![]));
        assert_eq!(
            only(read("SELECT title FROM book").unwrap()),
            vec![json!({ "title": "Seed" })]
        );
        for sql in [
            "INSERT INTO book VALUES ('x', 'y')",
            "UPDATE book SET title = 'z'",
            "DELETE FROM book",
            "DROP TABLE book",
            "CREATE TABLE t (x)",
            "PRAGMA user_version = 5",
            "PRAGMA query_only = OFF",
            "WITH x AS (SELECT 1) INSERT INTO book VALUES ('q', 'r')",
        ] {
            assert!(read(sql).is_err(), "{sql}");
        }
        // The refused statements changed nothing, and a read still works.
        assert_eq!(
            only(read("SELECT count(*) AS n FROM book").unwrap()),
            vec![json!({ "n": 1 })]
        );
    }

    #[test]
    fn each_file_is_its_own_database() {
        let dir = tempfile::tempdir().unwrap();
        let a = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        let b = ExtensionStore::open(&dir.path().join("b.sqlite")).unwrap();
        a.migrate(&migrations()).unwrap();
        assert!(b.write(&[stmt("SELECT * FROM book", vec![])]).is_err());
        // Nor can a statement reach another file or the main database.
        assert!(a
            .write(&[stmt("ATTACH DATABASE ?1 AS other", vec![json!("x.sqlite")])])
            .is_err());
    }

    fn main_db(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("main.sqlite");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE x (v TEXT); INSERT INTO x VALUES ('main');")
            .unwrap();
        path
    }

    fn main_value(path: &Path) -> String {
        Connection::open(path)
            .unwrap()
            .query_row("SELECT v FROM x", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_migration_cannot_attach_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_db(dir.path());
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        let sql = format!(
            "COMMIT; ATTACH '{}' AS m; UPDATE m.x SET v = 'pwned'; BEGIN;",
            main.display()
        );
        assert!(store.migrate(&[("V1__evil.sql".into(), sql)]).is_err());
        assert_eq!(main_value(&main), "main");
        // Nothing stays attached on the pooled connections.
        for _ in 0..2 {
            assert!(store.read(&stmt("SELECT v FROM m.x", vec![])).is_err());
            assert!(store
                .write(&[stmt("UPDATE m.x SET v = 'pwned'", vec![])])
                .is_err());
        }
        assert_eq!(main_value(&main), "main");
    }

    #[test]
    fn nothing_can_attach_even_without_a_statement_check() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_db(dir.path());
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        let conn = store.pool.get().unwrap();
        let sql = format!("ATTACH '{}' AS m", main.display());
        assert!(conn.execute_batch(&sql).is_err());
    }

    #[test]
    fn a_write_cannot_end_or_start_a_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        store.migrate(&migrations()).unwrap();
        for sql in [
            "COMMIT",
            "ROLLBACK",
            "BEGIN",
            "SAVEPOINT s",
            "RELEASE s",
            "END",
        ] {
            let result = store.write(&[
                stmt("INSERT INTO book VALUES ('w', 'W')", vec![]),
                stmt(sql, vec![]),
            ]);
            assert!(result.is_err(), "{sql}");
            let n = store
                .read(&stmt("SELECT count(*) AS n FROM book", vec![]))
                .unwrap();
            assert_eq!(only(n), vec![json!({ "n": 1 })], "{sql}");
        }
        // A plain write still works afterwards.
        store
            .write(&[stmt("INSERT INTO book VALUES ('w', 'W')", vec![])])
            .unwrap();
    }

    #[test]
    fn a_migration_cannot_control_transactions() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExtensionStore::open(&dir.path().join("a.sqlite")).unwrap();
        let bad = vec![(
            "V1__t.sql".to_string(),
            "CREATE TABLE t (x); COMMIT; BEGIN;".to_string(),
        )];
        assert!(store.migrate(&bad).is_err());
        let good = vec![("V1__t.sql".to_string(), "CREATE TABLE t (x);".to_string())];
        store.migrate(&good).unwrap();
    }
}
