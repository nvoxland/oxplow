//! Terminals as rows (`terminal`, migration `V106__terminal.sql`): a shell
//! in a stream's worktree. Every write is an `oxplow.terminal.*` command,
//! so the writes are `_tx` cores on the command's transaction; the plain
//! store only reads. A terminal's title is auto-numbered "Terminal N" when
//! none is given — one past the highest numbered title in its stream, and
//! never below the count, so a generated title never repeats after renames.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, StreamId, TerminalId, Timestamp};

use crate::database::{map_sql_err, string_to_ts, ts_to_string, Database};

/// One terminal of a stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Terminal {
    pub id: TerminalId,
    pub stream_id: StreamId,
    pub title: String,
    #[specta(type = oxplow_domain::JsNumber)]
    pub position: i64,
    pub created_at: Timestamp,
}

const COLS: &str = "id, stream_id, title, position, created_at";

fn row_to_terminal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Terminal> {
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Terminal {
        id: TerminalId::new(row.get(0)?),
        stream_id: StreamId::new(row.get(1)?),
        title: row.get(2)?,
        position: row.get(3)?,
        created_at: string_to_ts(&row.get::<_, String>(4)?).map_err(map_err)?,
    })
}

#[derive(Clone)]
pub struct SqliteTerminalStore {
    db: Database,
}

impl SqliteTerminalStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Terminal `id`.
    pub async fn get(&self, id: TerminalId) -> Result<Option<Terminal>, DomainError> {
        self.db.read(move |conn| terminal_tx(conn, id)).await
    }

    /// A stream's terminals in order.
    pub async fn for_stream(&self, stream: StreamId) -> Result<Vec<Terminal>, DomainError> {
        self.db.read(move |conn| for_stream_tx(conn, stream)).await
    }
}

/// Terminal `id`, on `conn`.
pub fn terminal_tx(conn: &Connection, id: TerminalId) -> Result<Option<Terminal>, DomainError> {
    let sql = format!("SELECT {COLS} FROM terminal WHERE id = ?1");
    let mut stmt = conn.prepare(&sql).map_err(map_sql_err)?;
    let mut rows = stmt
        .query_map(params![id.value()], row_to_terminal)
        .map_err(map_sql_err)?;
    rows.next().transpose().map_err(map_sql_err)
}

/// A stream's terminals in order, on `conn`.
pub fn for_stream_tx(conn: &Connection, stream: StreamId) -> Result<Vec<Terminal>, DomainError> {
    let sql = format!("SELECT {COLS} FROM terminal WHERE stream_id = ?1 ORDER BY position, id");
    let mut stmt = conn.prepare(&sql).map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![stream.value()], row_to_terminal)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// The next "Terminal N" title among `titles`: one past the highest
/// numbered one, and never below the count.
pub fn next_title<'a>(titles: impl IntoIterator<Item = &'a str>) -> String {
    let (mut count, mut highest) = (0usize, 0usize);
    for title in titles {
        count += 1;
        if let Some(n) = title
            .trim()
            .strip_prefix("Terminal ")
            .and_then(|n| n.parse::<usize>().ok())
        {
            highest = highest.max(n);
        }
    }
    format!("Terminal {}", highest.max(count) + 1)
}

/// A new terminal at the end of `stream`'s, titled `title` or the next
/// "Terminal N".
pub fn open_tx(
    conn: &Connection,
    stream: StreamId,
    title: Option<&str>,
) -> Result<Terminal, DomainError> {
    let existing = for_stream_tx(conn, stream)?;
    let title = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => next_title(existing.iter().map(|t| t.title.as_str())),
    };
    let position = existing
        .iter()
        .map(|t| t.position)
        .max()
        .map_or(0, |p| p + 1);
    conn.execute(
        "INSERT INTO terminal (stream_id, title, position, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            stream.value(),
            title,
            position,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    let id = TerminalId::new(conn.last_insert_rowid());
    Ok(terminal_tx(conn, id)?.expect("just opened"))
}

/// Give terminal `id` another title.
pub fn rename_tx(conn: &Connection, id: TerminalId, title: &str) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE terminal SET title = ?2 WHERE id = ?1",
        params![id.value(), title],
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// Delete terminal `id`.
pub fn delete_tx(conn: &Connection, id: TerminalId) -> Result<(), DomainError> {
    conn.execute("DELETE FROM terminal WHERE id = ?1", params![id.value()])
        .map(|_| ())
        .map_err(map_sql_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_number_past_the_highest_and_the_count() {
        assert_eq!(next_title([]), "Terminal 1");
        assert_eq!(next_title(["Terminal 1"]), "Terminal 2");
        // A rename never makes the next title repeat one in use.
        assert_eq!(
            next_title(["Terminal 1", "build", "Terminal 3"]),
            "Terminal 4"
        );
        assert_eq!(next_title(["a", "b", "c"]), "Terminal 4");
    }
}
