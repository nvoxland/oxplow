//! `oxplow-dev task move-to-oxtask`: the one-off move of core's tasks into
//! the oxtask extension's own file and bundled's fields.
//!
//! Run with oxplow quit, after the new binary has booted once (so the
//! migrations ran and bundled's `task_record` model and `request_task_next`
//! entity tables exist). Core's rows (`oxtask`, `oxtask_link`,
//! `oxtask_note`, `task_assignment`, `request_task`) are only read; nothing
//! of core's is deleted.
//!
//! - Each task becomes a row of the store's `task` with `n` equal to its old
//!   id, so `tsk<N>` keeps its id and its URI (the model's `store_param` is
//!   `provider`, the store `oxtask`).
//! - Links and notes go to `task_link` and `task_comment` (a note keeps its
//!   number).
//! - Assignments are `oxplow` fields of the task (`assignee`, `rank`),
//!   stored as `oxplow.model.set_fields` stores them.
//! - Request links are rows of bundled's `request_task_next` entity (an old
//!   basis `command` is `edited` there).
//!
//! The main database commits first (every write in it is idempotent), then
//! the store; a second run is refused once the store has tasks.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use oxplow_db::Database;
use oxplow_domain::DomainError;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

/// oxtask's first migration: the move writes the tables it makes. (Later
/// migrations are applied by the extension's own boot.)
const STORE_MIGRATIONS: &[(&str, &str)] = &[(
    "V1__tasks.sql",
    include_str!("../../../extensions/oxtask/storage/migrations/V1__tasks.sql"),
)];

const STORE: &str = "oxtask";
const TASK_RESOURCE: &str = "oxplow/task";
const MODEL: &str = "oxplow/task_record";
const MODEL_TABLE: &str = "model__oxplow__task_record";
const ENTITY_TABLE: &str = "ext__oxplow__request_task_next";
const PERSON: &str = "oxplow:oxplow/person";

struct Task {
    n: i64,
    title: String,
    body: String,
    stage: String,
    parent: Option<String>,
    priority: String,
    author: Option<String>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    deleted: bool,
}

struct Link {
    task_id: String,
    to_uri: String,
    link_type: String,
}

struct Note {
    n: i64,
    task_id: String,
    author: Option<String>,
    at: String,
    body: String,
}

/// What the main database gave, for the store's half.
struct Moved {
    tasks: Vec<Task>,
    links: Vec<Link>,
    notes: Vec<Note>,
    task_seq: i64,
    note_seq: i64,
    assignments: usize,
    assigned: usize,
    requests: BTreeMap<String, usize>,
}

fn task_uri(n: i64) -> String {
    oxplow_domain::uri::build::of(
        TASK_RESOURCE,
        &[("provider", STORE), ("id", &format!("tsk{n}"))],
    )
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Move core's tasks (see the module doc); `dry_run` does everything and
/// keeps nothing.
pub async fn move_to_oxtask(project: &Path, dry_run: bool) -> Result<String, String> {
    let layout = oxplow_app::AppLayout::for_project(project);
    let store_file = layout
        .project_dir
        .join(".oxplow/extensions-data")
        .join(format!("{STORE}.sqlite"));
    let existed = store_file.exists();
    let result = run(&layout.state_db_path, &store_file, dry_run).await;
    if dry_run && !existed {
        for suffix in ["", "-wal", "-shm"] {
            let mut f = store_file.clone().into_os_string();
            f.push(suffix);
            let _ = std::fs::remove_file(f);
        }
    }
    result
}

async fn run(db_path: &Path, store_file: &Path, dry_run: bool) -> Result<String, String> {
    let db = Database::open_existing(db_path).map_err(|e| format!("{}: {e}", db_path.display()))?;
    // The store, made as the extension would make it.
    let migrations: Vec<(String, String)> = STORE_MIGRATIONS
        .iter()
        .map(|(f, s)| (f.to_string(), s.to_string()))
        .collect();
    {
        let store = oxplow_db::extension_store::ExtensionStore::open(store_file).map_err(err)?;
        store.migrate(&migrations).map_err(err)?;
    }
    let mut conn = Connection::open(store_file).map_err(err)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(err)?;
    let have: i64 = conn
        .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
        .map_err(err)?;
    if have > 0 {
        return Err(format!(
            "oxtask's file already has {have} tasks: the move runs once, into an empty store"
        ));
    }
    // The main database: fields and request links, and the rows to move.
    let moved = if dry_run {
        db.rehearse(main_half).await
    } else {
        db.transaction(main_half).await
    }
    .map_err(|e| {
        let e = e.to_string();
        e.strip_prefix("invalid value: ").unwrap_or(&e).to_string()
    })?;
    let tx = conn.transaction().map_err(err)?;
    write_store(&tx, &moved).map_err(err)?;
    if dry_run {
        tx.rollback().map_err(err)?;
    } else {
        tx.commit().map_err(err)?;
    }
    let deleted = moved.tasks.iter().filter(|t| t.deleted).count();
    let requests: Vec<String> = moved
        .requests
        .iter()
        .map(|(b, n)| format!("{b} {n}"))
        .collect();
    Ok(format!(
        "{}{} tasks ({} soft-deleted), {} links, {} notes, {} assignments ({} with an assignee), \
         {} request links ({})",
        if dry_run {
            "dry run, nothing kept: "
        } else {
            "moved "
        },
        moved.tasks.len(),
        deleted,
        moved.links.len(),
        moved.notes.len(),
        moved.assignments,
        moved.assigned,
        moved.requests.values().sum::<usize>(),
        requests.join(", ")
    ))
}

fn table_exists(tx: &Transaction<'_>, name: &str) -> Result<bool, DomainError> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |r| r.get(0),
    )
    .map_err(sql)
}

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Invalid(e.to_string())
}

/// Read core's tables and write the fields and the entity rows.
fn main_half(tx: &Transaction<'_>) -> Result<Moved, DomainError> {
    if !table_exists(tx, MODEL_TABLE)? || !table_exists(tx, ENTITY_TABLE)? {
        return Err(DomainError::Invalid(
            "bundled's task model or its request_task_next entity is missing: \
             boot the new oxplow once first"
                .into(),
        ));
    }
    // The thread of each task's earliest linked request, for an agent's tasks.
    let mut thread_of: HashMap<String, String> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT rt.task_uri, r.thread_id FROM request_task rt
                   JOIN request r ON r.id = rt.request_id
                  ORDER BY rt.at, rt.request_id",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(sql)?;
        for row in rows {
            let (uri, thread) = row.map_err(sql)?;
            thread_of
                .entry(uri)
                .or_insert_with(|| format!("oxplow:oxplow/thread?id=thr{thread}"));
        }
    }
    let author = |n: i64, who: Option<String>| -> Option<String> {
        match who.as_deref() {
            Some("user") => Some(PERSON.to_string()),
            Some("agent") => thread_of.get(&task_uri(n)).cloned(),
            Some(other) => Some(other.to_string()),
            None => None,
        }
    };
    let mut tasks = Vec::new();
    {
        let mut st = tx
            .prepare(
                "SELECT id, parent_id, title, description, status, priority, created_at,
                        updated_at, completed_at, deleted_at IS NOT NULL, author
                   FROM oxtask ORDER BY id",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, bool>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (
                n,
                parent,
                title,
                body,
                stage,
                priority,
                created_at,
                updated_at,
                closed_at,
                deleted,
                who,
            ) = row.map_err(sql)?;
            tasks.push(Task {
                n,
                title,
                body,
                stage,
                parent: parent.map(|p| format!("tsk{p}")),
                priority,
                author: author(n, who),
                created_at,
                updated_at,
                closed_at,
                deleted,
            });
        }
    }
    let task_author: HashMap<i64, Option<String>> =
        tasks.iter().map(|t| (t.n, t.author.clone())).collect();
    let mut links = Vec::new();
    {
        let mut st = tx
            .prepare("SELECT from_item_id, to_uri, link_type FROM oxtask_link ORDER BY id")
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok(Link {
                    task_id: format!("tsk{}", r.get::<_, i64>(0)?),
                    to_uri: r.get(1)?,
                    link_type: r.get(2)?,
                })
            })
            .map_err(sql)?;
        for row in rows {
            links.push(row.map_err(sql)?);
        }
    }
    let mut notes = Vec::new();
    {
        let mut st = tx
            .prepare("SELECT id, task_id, author, created_at, body FROM oxtask_note ORDER BY id")
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (n, task, who, at, body) = row.map_err(sql)?;
            let who = match who.as_deref() {
                Some("user") => Some(PERSON.to_string()),
                // An agent's note: the thread its task's author is, if any.
                Some("agent") => task_author.get(&task).cloned().flatten(),
                other => other.map(str::to_string),
            };
            notes.push(Note {
                n,
                task_id: format!("tsk{task}"),
                author: who,
                at,
                body,
            });
        }
    }
    let seq = |name: &str| -> Result<i64, DomainError> {
        Ok(tx
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?
            .unwrap_or(0))
    };
    let task_seq = seq("oxtask")?.max(tasks.iter().map(|t| t.n).max().unwrap_or(0));
    let note_seq = seq("oxtask_note")?.max(notes.iter().map(|n| n.n).max().unwrap_or(0));

    // Assignments: the item's `oxplow` fields, as `set_fields` stores them
    // (a JSON value as text); a nobody keeps its rank.
    let mut assignments = 0;
    let mut assigned = 0;
    {
        let mut st = tx
            .prepare("SELECT uri, assignee, rank FROM task_assignment ORDER BY uri")
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })
            .map_err(sql)?;
        let rows: Vec<_> = rows.collect::<Result<_, _>>().map_err(sql)?;
        for (uri, assignee, rank) in rows {
            let mut fields = BTreeMap::new();
            if let Some(a) = assignee {
                fields.insert("assignee".to_string(), Value::String(a).to_string());
                assigned += 1;
            }
            let rank = if rank.fract() == 0.0 && rank.abs() < 1e15 {
                Value::from(rank as i64)
            } else {
                Value::from(rank)
            };
            fields.insert("rank".to_string(), rank.to_string());
            oxplow_db::model_store::put_fields_tx(tx, MODEL, &uri, "oxplow", &fields)?;
            assignments += 1;
        }
    }

    // Request links: `command` is `edited` in bundled's entity.
    let mut requests: BTreeMap<String, usize> = BTreeMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT rt.request_id, rt.task_uri, rt.basis, r.thread_id
                   FROM request_task rt JOIN request r ON r.id = rt.request_id
                  ORDER BY rt.request_id, rt.task_uri, rt.basis",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(sql)?;
        let rows: Vec<_> = rows.collect::<Result<_, _>>().map_err(sql)?;
        for (request, task, basis, thread) in rows {
            let request = oxplow_domain::uri::build::of(
                "oxplow/request",
                &[("id", &format!("req{request}"))],
            );
            let basis = if basis == "command" {
                "edited".to_string()
            } else {
                basis
            };
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO {ENTITY_TABLE} (key, request, task, basis, thread_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)"
                ),
                params![
                    format!("{request}|{task}|{basis}"),
                    request,
                    task,
                    basis,
                    thread
                ],
            )
            .map_err(sql)?;
            *requests.entry(basis).or_default() += 1;
        }
    }
    Ok(Moved {
        tasks,
        links,
        notes,
        task_seq,
        note_seq,
        assignments,
        assigned,
        requests,
    })
}

/// Write the store, in the caller's transaction. Parents may come after
/// their children in the file order, so the check waits for the commit.
fn write_store(tx: &Transaction<'_>, moved: &Moved) -> rusqlite::Result<()> {
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    for t in &moved.tasks {
        tx.execute(
            "INSERT INTO task (n, title, body, stage, parent, priority, author, created_at,
                               updated_at, closed_at, deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                t.n,
                t.title,
                t.body,
                t.stage,
                t.parent,
                t.priority,
                t.author,
                t.created_at,
                t.updated_at,
                t.closed_at,
                t.deleted
            ],
        )?;
    }
    for l in &moved.links {
        tx.execute(
            "INSERT INTO task_link (task_id, to_uri, type) VALUES (?1, ?2, ?3)",
            params![l.task_id, l.to_uri, l.link_type],
        )?;
    }
    for n in &moved.notes {
        tx.execute(
            "INSERT INTO task_comment (n, task_id, author, at, body) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![n.n, n.task_id, n.author, n.at, n.body],
        )?;
    }
    // A number that was used never comes back: the highest the old tables
    // ever gave, not only the highest still there.
    for (name, seq) in [("task", moved.task_seq), ("task_comment", moved.note_seq)] {
        let has: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = ?1)",
            [name],
            |r| r.get(0),
        )?;
        if has {
            tx.execute(
                "UPDATE sqlite_sequence SET seq = max(seq, ?2) WHERE name = ?1",
                params![name, seq],
            )?;
        } else {
            tx.execute(
                "INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)",
                params![name, seq],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success());
        let svc = oxplow_app::Services::boot(
            oxplow_app::AppLayout::for_project(dir.path()),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .unwrap();
        svc.streams.ensure_primary().await.unwrap();
        drop(svc);
        // What bundled's first boot makes (the test project loads no
        // extensions): the model's table and the entity's.
        let c = main_db(dir.path());
        c.execute_batch(&format!(
            "CREATE TABLE {MODEL_TABLE} (uri TEXT PRIMARY KEY);
             CREATE TABLE {ENTITY_TABLE} (key TEXT PRIMARY KEY, request TEXT, task TEXT, basis TEXT, thread_id INTEGER);"
        ))
        .unwrap();
        dir
    }

    fn main_db(dir: &Path) -> Connection {
        let c = Connection::open(oxplow_app::AppLayout::for_project(dir).state_db_path).unwrap();
        // The seed names tasks core's projection `task` doesn't hold.
        c.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        c
    }

    fn store(dir: &Path) -> Connection {
        Connection::open(dir.join(".oxplow/extensions-data/oxtask.sqlite")).unwrap()
    }

    /// A small core dataset: task 3 is a child of 2 and comes after it; 5 is
    /// soft deleted; 9 is the highest number the table ever gave. One link,
    /// two notes, two assignments (one to nobody), request links on 2 and 3.
    fn seed(dir: &Path) {
        let c = main_db(dir);
        let stream: i64 = c
            .query_row("SELECT id FROM streams LIMIT 1", [], |r| r.get(0))
            .unwrap();
        c.execute_batch(&format!(
            "INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (7, {stream}, 't', 'x', 'x');
             INSERT INTO request (id, thread_id) VALUES (11, 7), (12, 7);
             INSERT INTO oxtask (id, parent_id, title, description, status, priority, created_by, created_at, updated_at, completed_at, deleted_at, author) VALUES
               (2, NULL, 'Epic', 'about it', 'in_progress', 'high', 'user', 'c2', 'u2', NULL, NULL, 'user'),
               (3, 2, 'Child', '', 'done', 'medium', 'agent', 'c3', 'u3', 'd3', NULL, 'agent'),
               (5, NULL, 'Gone', '', 'canceled', 'low', 'agent', 'c5', 'u5', NULL, 'x5', 'agent');
             UPDATE sqlite_sequence SET seq = 9 WHERE name = 'oxtask';
             INSERT INTO oxtask_link (from_item_id, to_uri, link_type, created_at) VALUES
               (3, '{u2}', 'blocks', 'l');
             INSERT INTO oxtask_note (id, task_id, body, author, created_at) VALUES
               (390, 3, 'agent note', 'agent', 'n1'), (391, 2, 'user note', 'user', 'n2');
             INSERT INTO task_assignment (uri, assignee, rank, assigned_at) VALUES
               ('{u2}', 'oxplow:oxplow/thread?id=thr7', 2.0, 'a'),
               ('{u3}', NULL, 1.5, 'a');
             INSERT INTO request_task (request_id, task_uri, basis, turn_id, at) VALUES
               (11, '{u3}', 'command', NULL, 'r1'),
               (12, '{u3}', 'assigned', NULL, 'r2'),
               (12, '{u2}', 'transition', NULL, 'r3');",
            u2 = task_uri(2),
            u3 = task_uri(3),
        ))
        .unwrap();
    }

    type TaskRow = (
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
        Option<String>,
    );

    #[tokio::test]
    async fn core_tasks_move_into_the_store_the_fields_and_the_entity() {
        let dir = project().await;
        seed(dir.path());
        let said = move_to_oxtask(dir.path(), false).await.unwrap();
        assert!(said.contains("3 tasks (1 soft-deleted), 1 links, 2 notes, 2 assignments (1 with an assignee), 3 request links"), "{said}");
        let s = store(dir.path());
        let rows: Vec<TaskRow> = s
            .prepare("SELECT n, id, title, stage, parent, author, closed_at, deleted, body FROM task ORDER BY n")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows[0],
            (
                2,
                "tsk2".into(),
                "Epic".into(),
                "in_progress".into(),
                None,
                Some(PERSON.into()),
                None,
                false,
                Some("about it".into())
            )
        );
        // An agent's task: the thread of its earliest request (11).
        assert_eq!(rows[1].4.as_deref(), Some("tsk2"));
        assert_eq!(rows[1].5.as_deref(), Some("oxplow:oxplow/thread?id=thr7"));
        assert_eq!(rows[1].6.as_deref(), Some("d3"));
        // No request: no author. Soft deleted stays a row.
        assert_eq!(rows[2].5, None);
        assert!(rows[2].7);
        let (c3, u3, pr): (String, String, String) = s
            .query_row(
                "SELECT created_at, updated_at, priority FROM task WHERE n = 3",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (c3.as_str(), u3.as_str(), pr.as_str()),
            ("c3", "u3", "medium")
        );
        let link: (String, String, String) = s
            .query_row("SELECT task_id, to_uri, type FROM task_link", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(link, ("tsk3".into(), task_uri(2), "blocks".into()));
        let notes: Vec<(i64, String, Option<String>, String, String)> = s
            .prepare("SELECT n, task_id, author, at, body FROM task_comment ORDER BY n")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            notes[0],
            (
                390,
                "tsk3".into(),
                Some("oxplow:oxplow/thread?id=thr7".into()),
                "n1".into(),
                "agent note".into()
            )
        );
        assert_eq!(notes[1].2.as_deref(), Some(PERSON));
        let seq = |name: &str| -> i64 {
            s.query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(seq("task"), 9);
        assert_eq!(seq("task_comment"), 391);
        // The next task takes the next number.
        s.execute("INSERT INTO task (title) VALUES ('new')", [])
            .unwrap();
        let n: i64 = s
            .query_row("SELECT max(n) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 10);
        // A hard delete is still refused.
        assert!(s.execute("DELETE FROM task WHERE n = 2", []).is_err());

        let m = main_db(dir.path());
        let fields: Vec<(String, String, String)> = m
            .prepare("SELECT uri, name, value FROM model_field_value WHERE model = 'oxplow/task_record' AND namespace = 'oxplow' ORDER BY uri, name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            fields,
            vec![
                (
                    task_uri(2),
                    "assignee".into(),
                    "\"oxplow:oxplow/thread?id=thr7\"".into()
                ),
                (task_uri(2), "rank".into(), "2".into()),
                (task_uri(3), "rank".into(), "1.5".into()),
            ]
        );
        let entity: Vec<(String, String, String, String, i64)> = m
            .prepare(&format!(
                "SELECT key, request, task, basis, thread_id FROM {ENTITY_TABLE} ORDER BY key"
            ))
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let req =
            |n: i64| oxplow_domain::uri::build::of("oxplow/request", &[("id", &format!("req{n}"))]);
        assert_eq!(entity.len(), 3);
        assert!(entity.contains(&(
            format!("{}|{}|edited", req(11), task_uri(3)),
            req(11),
            task_uri(3),
            "edited".into(),
            7
        )));
        assert!(entity.iter().any(|e| e.3 == "assigned" && e.1 == req(12)));
        assert!(entity
            .iter()
            .any(|e| e.3 == "transition" && e.2 == task_uri(2)));

        // A second run is refused (the store has the three and the new one).
        let e = move_to_oxtask(dir.path(), false).await.unwrap_err();
        assert!(e.contains("already has 4 tasks"), "{e}");
    }

    #[tokio::test]
    async fn a_dry_run_keeps_nothing() {
        let dir = project().await;
        seed(dir.path());
        let said = move_to_oxtask(dir.path(), true).await.unwrap();
        assert!(said.starts_with("dry run, nothing kept: 3 tasks"), "{said}");
        assert!(!dir
            .path()
            .join(".oxplow/extensions-data/oxtask.sqlite")
            .exists());
        let m = main_db(dir.path());
        let n: i64 = m
            .query_row(
                "SELECT count(*) FROM model_field_value WHERE model = 'oxplow/task_record'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        let n: i64 = m
            .query_row(&format!("SELECT count(*) FROM {ENTITY_TABLE}"), [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
        // The real run still works after it.
        move_to_oxtask(dir.path(), false).await.unwrap();
    }

    #[tokio::test]
    async fn a_project_that_never_booted_the_new_oxplow_is_refused() {
        let dir = project().await;
        main_db(dir.path())
            .execute_batch(&format!("DROP TABLE {ENTITY_TABLE}"))
            .unwrap();
        let e = move_to_oxtask(dir.path(), false).await.unwrap_err();
        assert!(e.contains("boot the new oxplow once first"), "{e}");
    }
}
