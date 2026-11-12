//! The store's two ways into the database: a read (a rolled-back
//! snapshot) and a write (one transaction, retried on `SQLITE_BUSY`),
//! rusqlite errors mapped to the domain's.

use oxplow_db::{map_sql_err, Database};
use oxplow_domain::DomainError;

pub(crate) async fn read<R, F>(db: &Database, f: F) -> Result<R, DomainError>
where
    F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<R> + Send + 'static,
    R: Send + 'static,
{
    db.read(move |tx| f(tx).map_err(map_sql_err)).await
}

pub(crate) async fn write<R, F>(db: &Database, f: F) -> Result<R, DomainError>
where
    F: Fn(&rusqlite::Connection) -> rusqlite::Result<R> + Send + 'static,
    R: Send + 'static,
{
    db.transaction(move |tx| f(tx).map_err(map_sql_err)).await
}
