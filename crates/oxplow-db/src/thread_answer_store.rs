//! An agent's answers in a thread (`thread_answer` → `v_thread_answer`,
//! P6.C1): what `lens.show` recorded, and what `lens.keep` made of it.
//! Written inside the commands' transactions; read here and as the model.

use oxplow_domain::{DomainError, Timestamp};
use rusqlite::OptionalExtension;

use crate::database::{map_sql_err, ts_to_string};
use crate::Database;

/// What an answer shows: an existing lens (with params), or its own spec.
#[derive(Debug, Clone, PartialEq)]
pub enum AnswerShows {
    Lens(String),
    Spec(serde_json::Value),
}

/// A recorded answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadAnswer {
    pub id: i64,
    pub thread_id: i64,
    pub turn_id: Option<i64>,
    pub effort_id: Option<i64>,
    pub title: String,
    pub shows: AnswerShows,
    pub params: serde_json::Value,
    pub created_at: String,
    pub kept_lens: Option<String>,
}

/// The turn and effort open in `thread` now — what an answer shown in it
/// belongs to.
pub fn open_turn_and_effort_tx(
    conn: &rusqlite::Connection,
    thread: i64,
) -> Result<(Option<i64>, Option<i64>), DomainError> {
    let turn = conn
        .query_row(
            "SELECT id FROM agent_turn WHERE thread_id = ?1 AND ended_at IS NULL
             ORDER BY id DESC LIMIT 1",
            [thread],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    let effort = conn
        .query_row(
            "SELECT id FROM effort WHERE thread_id = ?1 AND ended_at IS NULL
             ORDER BY id DESC LIMIT 1",
            [thread],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    Ok((turn, effort))
}

/// Record an answer; its id.
pub fn insert_tx(
    conn: &rusqlite::Connection,
    thread: i64,
    title: &str,
    shows: &AnswerShows,
    params: &serde_json::Value,
) -> Result<i64, DomainError> {
    let (turn, effort) = open_turn_and_effort_tx(conn, thread)?;
    let (lens, spec) = match shows {
        AnswerShows::Lens(id) => (Some(id.clone()), None),
        AnswerShows::Spec(spec) => (None, Some(spec.to_string())),
    };
    conn.execute(
        "INSERT INTO thread_answer (thread_id, turn_id, effort_id, title, lens, spec, params,
                                    created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            thread,
            turn,
            effort,
            title,
            lens,
            spec,
            params.to_string(),
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

pub fn get_tx(conn: &rusqlite::Connection, id: i64) -> Result<Option<ThreadAnswer>, DomainError> {
    conn.query_row(
        "SELECT id, thread_id, turn_id, effort_id, title, lens, spec, params, created_at,
                kept_lens
         FROM thread_answer WHERE id = ?1",
        [id],
        |r| {
            let lens: Option<String> = r.get(5)?;
            let spec: Option<String> = r.get(6)?;
            let params: String = r.get(7)?;
            Ok(ThreadAnswer {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                turn_id: r.get(2)?,
                effort_id: r.get(3)?,
                title: r.get(4)?,
                shows: match (lens, spec) {
                    (Some(l), _) => AnswerShows::Lens(l),
                    (None, s) => AnswerShows::Spec(
                        s.and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(serde_json::Value::Null),
                    ),
                },
                params: serde_json::from_str(&params).unwrap_or_default(),
                created_at: r.get(8)?,
                kept_lens: r.get(9)?,
            })
        },
    )
    .optional()
    .map_err(map_sql_err)
}

/// Note that answer `id` was kept as lens `lens`.
pub fn set_kept_tx(conn: &rusqlite::Connection, id: i64, lens: &str) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE thread_answer SET kept_lens = ?2 WHERE id = ?1",
        rusqlite::params![id, lens],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

#[derive(Clone)]
pub struct SqliteThreadAnswerStore {
    db: Database,
}

impl SqliteThreadAnswerStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: i64) -> Result<Option<ThreadAnswer>, DomainError> {
        self.db.read(move |c| get_tx(c, id)).await
    }
}
