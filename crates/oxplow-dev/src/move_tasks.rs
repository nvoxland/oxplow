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
//! Who wrote what comes from the event log, as the people-facing screens
//! read it: a task's author is the thread of its `task.created` event (else
//! of the request link its creation made — basis `command`, at its creation
//! time); a note's is the thread of the `task.commented` event naming it
//! (`not<N>`). A person's rows are the person. `author_kind` is core's old
//! author column (`user` is a person). A closed task's `closed_at` is core's
//! `task` row's, else its last update.
//!
//! The move takes the instance lock (it refuses while oxplow runs). The
//! main database commits first (every write in it is idempotent), then the
//! store; a second run is refused once the store has tasks, unless
//! `--replace`: that deletes the store file and the fields and entity rows
//! an earlier run wrote, then moves again.

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
    author_kind: Option<&'static str>,
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
    author_kind: Option<&'static str>,
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

/// A timestamp to the second (its first 19 characters).
fn second(at: &str) -> String {
    at.chars().take(19).collect()
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
pub async fn move_to_oxtask(
    project: &Path,
    dry_run: bool,
    replace: bool,
) -> Result<String, String> {
    let layout = oxplow_app::AppLayout::for_project(project);
    let running =
        "oxplow is running on this project: quit it first (the move runs as part of the flip)";
    if oxplow_app::is_project_locked(project) {
        return Err(running.into());
    }
    // Held for the whole move, as a daemon holds it.
    let _lock = oxplow_app::try_acquire_instance_lock(&layout)
        .map_err(err)?
        .ok_or(running)?;
    let real_file = layout
        .project_dir
        .join(".oxplow/extensions-data")
        .join(format!("{STORE}.sqlite"));
    // A rehearsal of a replace works on a copy of nothing: the real file is
    // not touched, and the fresh one is the empty store a replace leaves.
    let store_file = if dry_run && replace {
        remove_store(&real_file.with_extension("sqlite.rehearsal"));
        real_file.with_extension("sqlite.rehearsal")
    } else {
        real_file.clone()
    };
    if replace && !dry_run {
        remove_store(&store_file);
    }
    let existed = store_file.exists();
    let result = run(&layout.state_db_path, &store_file, dry_run, replace).await;
    if dry_run && !existed {
        remove_store(&store_file);
    }
    result
}

fn remove_store(store_file: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut f = store_file.to_path_buf().into_os_string();
        f.push(suffix);
        let _ = std::fs::remove_file(f);
    }
}

async fn run(
    db_path: &Path,
    store_file: &Path,
    dry_run: bool,
    replace: bool,
) -> Result<String, String> {
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
        db.rehearse(move |tx| main_half(tx, replace)).await
    } else {
        db.transaction(move |tx| main_half(tx, replace)).await
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
fn main_half(tx: &Transaction<'_>, replace: bool) -> Result<Moved, DomainError> {
    if !table_exists(tx, MODEL_TABLE)? || !table_exists(tx, ENTITY_TABLE)? {
        return Err(DomainError::Invalid(
            "bundled's task model or its request_task_next entity is missing: \
             boot the new oxplow once first, with oxplow-bundled enabled"
                .into(),
        ));
    }
    if replace {
        // What an earlier run wrote: oxplow's two fields of the task model
        // and the request-link entity (nothing of another namespace's).
        tx.execute(
            "DELETE FROM model_field_value WHERE model = ?1 AND namespace = 'oxplow'",
            [MODEL],
        )
        .map_err(sql)?;
        tx.execute(&format!("DELETE FROM {ENTITY_TABLE}"), [])
            .map_err(sql)?;
    }
    // Who made what, from the event log (`event_log_type_seq`): the thread of
    // a task's `task.created` event, and of each `task.commented` event by the
    // note it names. Events with no thread were a person's or the system's.
    let mut created_by: HashMap<String, i64> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT j.value, e.thread_id FROM event_log e, json_each(e.subject) j
                  WHERE e.type = 'task.created' AND e.thread_id IS NOT NULL ORDER BY e.seq",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(sql)?;
        for row in rows {
            let (uri, thread) = row.map_err(sql)?;
            created_by.entry(uri).or_insert(thread);
        }
    }
    let mut commented_by: HashMap<i64, i64> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT json_extract(payload, '$.comment'), thread_id FROM event_log
                  WHERE type = 'task.commented' AND thread_id IS NOT NULL ORDER BY seq",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(sql)?;
        for row in rows {
            let (comment, thread) = row.map_err(sql)?;
            if let Some(n) = comment
                .as_deref()
                .and_then(|c| c.strip_prefix("not"))
                .and_then(|n| n.parse::<i64>().ok())
            {
                commented_by.entry(n).or_insert(thread);
            }
        }
    }
    // The request link a create made (basis `command`, at its creation
    // time, to the second): a task filed without a `task.created` event
    // still has its author's thread there. A link of another basis or time
    // says nothing about who filed it.
    let mut filed_by: HashMap<(String, String), i64> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT rt.task_uri, rt.at, r.thread_id FROM request_task rt
                   JOIN request r ON r.id = rt.request_id
                  WHERE rt.basis = 'command' ORDER BY rt.at, rt.request_id",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (uri, at, thread) = row.map_err(sql)?;
            filed_by.entry((uri, second(&at))).or_insert(thread);
        }
    }
    // Core's `task` projection closes a task on done and on canceled; the
    // old table's `completed_at` only on done.
    let mut closed_in_core: HashMap<String, String> = HashMap::new();
    {
        let mut st = tx
            .prepare("SELECT uri, closed_at FROM task WHERE closed_at IS NOT NULL")
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(sql)?;
        for row in rows {
            let (uri, at) = row.map_err(sql)?;
            closed_in_core.insert(uri, at);
        }
    }
    let thread_uri = |thread: i64| format!("oxplow:oxplow/thread?id=thr{thread}");
    let author = |who: Option<&str>, thread: Option<i64>| -> Option<String> {
        match who {
            Some("user") => Some(PERSON.to_string()),
            Some("agent") => thread.map(thread_uri),
            Some(other) => Some(other.to_string()),
            None => None,
        }
    };
    let kind = |who: Option<&str>| -> Option<&'static str> {
        match who {
            Some("user") => Some("person"),
            Some("agent") => Some("agent"),
            Some("system") => Some("system"),
            _ => None,
        }
    };
    let mut tasks = Vec::new();
    {
        let mut st = tx
            .prepare(
                "SELECT id, parent_id, title, description, status, priority, created_at,
                        updated_at, deleted_at IS NOT NULL, author
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
                    r.get::<_, bool>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (n, parent, title, body, stage, priority, created_at, updated_at, deleted, who) =
                row.map_err(sql)?;
            let uri = task_uri(n);
            let thread = created_by
                .get(&uri)
                .or_else(|| filed_by.get(&(uri.clone(), second(&created_at))))
                .copied();
            let closed_at = matches!(stage.as_str(), "done" | "canceled").then(|| {
                closed_in_core
                    .get(&uri)
                    .cloned()
                    .unwrap_or_else(|| updated_at.clone())
            });
            tasks.push(Task {
                n,
                title,
                body,
                stage,
                parent: parent.map(|p| format!("tsk{p}")),
                priority,
                author: author(who.as_deref(), thread),
                author_kind: kind(who.as_deref()),
                created_at,
                updated_at,
                closed_at,
                deleted,
            });
        }
    }
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
            let _ = task;
            let author_kind = kind(who.as_deref());
            let who = author(who.as_deref(), commented_by.get(&n).copied());
            notes.push(Note {
                n,
                task_id: format!("tsk{task}"),
                author: who,
                author_kind,
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
            "INSERT INTO task (n, title, body, stage, parent, priority, author, author_kind,
                               created_at, updated_at, closed_at, deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                t.n,
                t.title,
                t.body,
                t.stage,
                t.parent,
                t.priority,
                t.author,
                t.author_kind,
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
            "INSERT INTO task_comment (n, task_id, author, author_kind, at, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![n.n, n.task_id, n.author, n.author_kind, n.at, n.body],
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
    /// soft deleted (and canceled); 9 is the highest number the table ever
    /// gave. One link, notes by a person and by agents, two assignments (one
    /// to nobody), request links, and the events that say who made what:
    /// task 3 was filed by thread 7's agent, task 6 by thread 8's command
    /// (no created event, the request link made at its creation), task 7 has
    /// only a later link (its author is unknown).
    fn seed(dir: &Path) {
        let c = main_db(dir);
        let stream: i64 = c
            .query_row("SELECT id FROM streams LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let event = |id: &str, kind: &str, thread: &str, subject: &str, payload: &str| {
            format!(
                "INSERT INTO event_log (id, type, v, at, source, thread_id, subject, payload)
                 VALUES ('{id}', '{kind}', 2, 't', 's', {thread}, '[\"{subject}\"]', '{payload}');"
            )
        };
        let (u2, u3) = (task_uri(2), task_uri(3));
        let created = event(
            "e1",
            "task.created",
            "7",
            &u3,
            &format!("{{\"task\":\"{u3}\",\"state\":\"todo\"}}"),
        );
        // Another task's created event must not be taken for task 3's.
        let other = event(
            "e2",
            "task.created",
            "8",
            &u2,
            &format!("{{\"task\":\"{u2}\",\"state\":\"todo\"}}"),
        );
        let commented = [
            event(
                "e3",
                "task.commented",
                "8",
                &u3,
                &format!("{{\"task\":\"{u3}\",\"comment\":\"not390\"}}"),
            ),
            event(
                "e4",
                "task.commented",
                "7",
                &u2,
                &format!("{{\"task\":\"{u2}\",\"comment\":\"not392\"}}"),
            ),
        ]
        .join("\n");
        c.execute_batch(&format!(
            "INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (7, {stream}, 't', 'x', 'x'), (8, {stream}, 'u', 'x', 'x');
             INSERT INTO request (id, thread_id) VALUES (11, 7), (12, 7), (13, 8);
             INSERT INTO oxtask (id, parent_id, title, description, status, priority, created_by, created_at, updated_at, completed_at, deleted_at, author) VALUES
               (2, NULL, 'Epic', 'about it', 'in_progress', 'high', 'user', 'c2', 'u2', NULL, NULL, 'user'),
               (3, 2, 'Child', '', 'done', 'medium', 'agent', 'c3', 'u3', 'd3', NULL, 'agent'),
               (5, NULL, 'Gone', '', 'canceled', 'low', 'agent', 'c5', 'u5', NULL, 'x5', 'agent'),
               (6, NULL, 'By command', '', 'ready', 'low', 'agent', 'c6', 'u6', NULL, NULL, 'agent'),
               (7, NULL, 'Linked later', '', 'ready', 'low', 'agent', 'c7', 'u7', NULL, NULL, 'agent');
             UPDATE sqlite_sequence SET seq = 9 WHERE name = 'oxtask';
             INSERT INTO task (uri, provider, title, state, stage, created_at, updated_at, closed_at) VALUES
               ('{u3}', 'oxtask', 'Child', 'done', 'done', 'c3', 'u3', 'cl3');
             {created}
             {other}
             {commented}
             INSERT INTO oxtask_link (from_item_id, to_uri, link_type, created_at) VALUES
               (3, '{u2}', 'blocks', 'l');
             INSERT INTO oxtask_note (id, task_id, body, author, created_at) VALUES
               (390, 3, 'agent note', 'agent', 'n1'), (391, 2, 'user note', 'user', 'n2'),
               (392, 2, 'agent note on a person task', 'agent', 'n3'),
               (393, 3, 'agent note with no event', 'agent', 'n4');
             INSERT INTO task_assignment (uri, assignee, rank, assigned_at) VALUES
               ('{u2}', 'oxplow:oxplow/thread?id=thr7', 2.0, 'a'),
               ('{u3}', NULL, 1.5, 'a');
             INSERT INTO request_task (request_id, task_uri, basis, turn_id, at) VALUES
               (11, '{u3}', 'command', NULL, 'r1'),
               (12, '{u3}', 'assigned', NULL, 'r2'),
               (12, '{u2}', 'transition', NULL, 'r3'),
               (13, '{u6}', 'command', NULL, 'c6'),
               (12, '{u7}', 'assigned', NULL, 'c7');",
            u6 = task_uri(6),
            u7 = task_uri(7),
        ))
        .unwrap();
    }

    type NoteRow = (i64, String, Option<String>, String, String, Option<String>);

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
        Option<String>,
    );

    #[tokio::test]
    async fn core_tasks_move_into_the_store_the_fields_and_the_entity() {
        let dir = project().await;
        seed(dir.path());
        let said = move_to_oxtask(dir.path(), false, false).await.unwrap();
        assert!(said.contains("5 tasks (1 soft-deleted), 1 links, 4 notes, 2 assignments (1 with an assignee), 5 request links"), "{said}");
        let s = store(dir.path());
        let rows: Vec<TaskRow> = s
            .prepare("SELECT n, id, title, stage, parent, author, closed_at, deleted, body, author_kind FROM task ORDER BY n")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)))
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
                Some("about it".into()),
                Some("person".into())
            )
        );
        // An agent's task: the thread of its `task.created` event (7), not
        // the other task's event (8). Its closed_at is core's projection's.
        assert_eq!(rows[1].4.as_deref(), Some("tsk2"));
        assert_eq!(rows[1].5.as_deref(), Some("oxplow:oxplow/thread?id=thr7"));
        assert_eq!(rows[1].6.as_deref(), Some("cl3"));
        assert_eq!(rows[1].9.as_deref(), Some("agent"));
        // No event, no request: no author (the kind stays). Soft deleted
        // stays a row; a canceled task with no completed_at is closed at
        // its last update.
        assert_eq!(rows[2].5, None);
        assert_eq!(rows[2].9.as_deref(), Some("agent"));
        assert!(rows[2].7);
        assert_eq!(rows[2].6.as_deref(), Some("u5"));
        // No event: the thread of the request link the create made (basis
        // `command`, at its creation time) — and never a link of another
        // basis or time.
        assert_eq!(rows[3].5.as_deref(), Some("oxplow:oxplow/thread?id=thr8"));
        assert_eq!(rows[4].5, None);
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
        let notes: Vec<NoteRow> = s
            .prepare(
                "SELECT n, task_id, author, at, body, author_kind FROM task_comment ORDER BY n",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            notes[0],
            (
                390,
                "tsk3".into(),
                Some("oxplow:oxplow/thread?id=thr8".into()),
                "n1".into(),
                "agent note".into(),
                Some("agent".into())
            )
        );
        assert_eq!(notes[1].2.as_deref(), Some(PERSON));
        assert_eq!(notes[1].5.as_deref(), Some("person"));
        // An agent's note on a person's task is the agent's, never the
        // person's; one with no event has no author.
        assert_eq!(notes[2].2.as_deref(), Some("oxplow:oxplow/thread?id=thr7"));
        assert_eq!(notes[2].5.as_deref(), Some("agent"));
        assert_eq!(notes[3].2, None);
        assert_eq!(notes[3].5.as_deref(), Some("agent"));
        let seq = |name: &str| -> i64 {
            s.query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(seq("task"), 9);
        assert_eq!(seq("task_comment"), 393);
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
        assert_eq!(entity.len(), 5);
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
        let e = move_to_oxtask(dir.path(), false, false).await.unwrap_err();
        assert!(e.contains("already has 6 tasks"), "{e}");
    }

    #[tokio::test]
    async fn a_dry_run_keeps_nothing() {
        let dir = project().await;
        seed(dir.path());
        let said = move_to_oxtask(dir.path(), true, false).await.unwrap();
        assert!(said.starts_with("dry run, nothing kept: 5 tasks"), "{said}");
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
        move_to_oxtask(dir.path(), false, false).await.unwrap();
    }

    #[tokio::test]
    async fn a_project_that_never_booted_the_new_oxplow_is_refused() {
        let dir = project().await;
        main_db(dir.path())
            .execute_batch(&format!("DROP TABLE {ENTITY_TABLE}"))
            .unwrap();
        let e = move_to_oxtask(dir.path(), false, false).await.unwrap_err();
        assert!(e.contains("boot the new oxplow once first"), "{e}");
    }

    #[tokio::test]
    async fn the_move_refuses_while_oxplow_holds_the_project() {
        let dir = project().await;
        seed(dir.path());
        let layout = oxplow_app::AppLayout::for_project(dir.path());
        let _held = oxplow_app::try_acquire_instance_lock(&layout)
            .unwrap()
            .unwrap();
        for dry_run in [true, false] {
            let e = move_to_oxtask(dir.path(), dry_run, false)
                .await
                .unwrap_err();
            assert!(e.contains("oxplow is running"), "{e}");
        }
    }

    #[tokio::test]
    async fn replace_redoes_the_move_over_what_a_first_run_left() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false).await.unwrap();
        // Something the old run left that the new one no longer makes, and
        // another namespace's field that is not the move's to touch.
        let m = main_db(dir.path());
        m.execute_batch(&format!(
            "INSERT INTO model_field_value (model, uri, namespace, name, value) VALUES
               ('oxplow/task_record', '{stale}', 'oxplow', 'assignee', '\"x\"'),
               ('oxplow/task_record', '{kept}', 'oxtask', 'priority', '\"high\"');
             INSERT INTO {ENTITY_TABLE} (key, request, task, basis, thread_id) VALUES ('stale|k', 'r', 't', 'edited', 1);",
            stale = task_uri(99),
            kept = task_uri(2),
        ))
        .unwrap();
        // Without --replace the second run is still refused.
        let e = move_to_oxtask(dir.path(), false, false).await.unwrap_err();
        assert!(e.contains("already has"), "{e}");
        // A dry run of it changes nothing on disk.
        let said = move_to_oxtask(dir.path(), true, true).await.unwrap();
        assert!(said.starts_with("dry run, nothing kept: 5 tasks"), "{said}");
        let have: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(have, 5);
        let said = move_to_oxtask(dir.path(), false, true).await.unwrap();
        assert!(said.starts_with("moved 5 tasks"), "{said}");
        let have: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(have, 5);
        let count = |sql: &str| -> i64 {
            main_db(dir.path())
                .query_row(sql, [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            count("SELECT count(*) FROM model_field_value WHERE namespace = 'oxplow' AND uri LIKE '%tsk99%'"),
            0
        );
        assert_eq!(
            count("SELECT count(*) FROM model_field_value WHERE namespace = 'oxtask'"),
            1
        );
        assert_eq!(count(&format!("SELECT count(*) FROM {ENTITY_TABLE}")), 5);
    }
}
