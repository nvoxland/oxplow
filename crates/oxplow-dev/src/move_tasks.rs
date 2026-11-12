//! `oxplow-dev task move-to-oxtask`: the one-off move of core's tasks into
//! the oxtask extension's own file and bundled's fields.
//!
//! Run with oxplow quit, after the new binary has booted once (so the
//! migrations ran and bundled's `task_record` and `request_task` model
//! tables exist). Core's rows (`oxtask`, `oxtask_link`,
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
//! - Request links are rows of bundled's `request_task` model, store
//!   `oxplow`, with core's `at` and `turn_id` (an old
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
//! `--replace`: that deletes the store file and the fields and request-link
//! rows an earlier run wrote, then moves again.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use oxplow_db::Database;
use oxplow_domain::DomainError;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

/// oxtask's first migration: the move writes the tables it makes. (Later
/// migrations are applied by the extension's own boot.)
pub(crate) const STORE_MIGRATIONS: &[(&str, &str)] = &[(
    "V1__tasks.sql",
    include_str!("../../../extensions/oxtask/storage/migrations/V1__tasks.sql"),
)];

const STORE: &str = "oxtask";
const TASK_RESOURCE: &str = "oxplow/task";
const MODEL: &str = "oxplow/task_record";
const MODEL_TABLE: &str = "model__oxplow__task_record";
const LINK_MODEL_TABLE: &str = "model__oxplow__request_task";
/// The table an earlier build of bundled made for this model while the flip
/// was in progress; the move drops it.
const LEGACY_LINK_MODEL_TABLE: &str = "model__oxplow__request_task_next";
const LINK_RESOURCE: &str = "oxplow/request_task";
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
    created_at: String,
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
    force_discard: bool,
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
    preconditions(project, &layout.state_db_path)?;
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
    if replace && real_file.exists() {
        // New work is not the move's to lose: a store task core's tables
        // lack was made since the first move.
        let lacking = tasks_core_lacks(&real_file, &layout.state_db_path)?;
        if !lacking.is_empty() && !force_discard {
            return Err(format!(
                "oxtask's file has {} task(s) core's tables lack ({}): `--replace` would \
                 lose them. Keep them, or pass `--force-discard` (the file is kept as a \
                 backup either way)",
                lacking.len(),
                lacking
                    .iter()
                    .take(10)
                    .map(|n| format!("tsk{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let mut kept = None;
    if !dry_run {
        copy_main_database(&layout.state_db_path)?;
        if replace && real_file.exists() {
            kept = Some(keep_as_backup(&real_file)?);
        }
    }
    let existed = store_file.exists();
    let result = run(&layout.state_db_path, &store_file, dry_run, replace).await;
    if dry_run && !existed {
        remove_store(&store_file);
    }
    if result.is_err() {
        if let Some(backup) = kept {
            // The move failed: what was there comes back, and the empty
            // store the failed run made goes.
            remove_store(&real_file);
            restore_backup(&real_file, &backup)?;
        }
    }
    result
}

/// What must hold before anything is renamed, written or deleted: the main
/// database opens and carries bundled's two model tables (a boot of the new
/// oxplow made them), and oxtask is enabled (the move fills its store).
fn preconditions(project: &Path, db_path: &Path) -> Result<(), String> {
    if !oxplow_config::extension_settings(project).is_enabled(STORE) {
        return Err(format!(
            "the {STORE} extension is off in this project: turn it on first (the move fills its store)"
        ));
    }
    let main = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{}: {e}", db_path.display()))?;
    for table in [MODEL_TABLE, LINK_MODEL_TABLE] {
        let there: bool = main
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !there {
            return Err("bundled's task or request_task model is missing: \
                        boot the new oxplow once first, with oxplow-bundled enabled"
                .into());
        }
    }
    Ok(())
}

/// A consistent copy of the main database next to it, made before the move
/// deletes anything from it: `local.sqlite.pre-move-<unix seconds>`.
fn copy_main_database(db_path: &Path) -> Result<std::path::PathBuf, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err)?
        .as_secs();
    // A second run in the same second gets its own file.
    let copy = (0u32..)
        .map(|n| {
            let mut name = db_path.as_os_str().to_owned();
            name.push(format!(".pre-move-{stamp}"));
            if n > 0 {
                name.push(format!("-{n}"));
            }
            std::path::PathBuf::from(name)
        })
        .find(|p| !p.exists())
        .expect("an unused name");
    let main = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    main.execute("VACUUM INTO ?1", [copy.to_string_lossy().as_ref()])
        .map_err(|e| format!("copying {}: {e}", db_path.display()))?;
    Ok(copy)
}

/// Put the files `keep_as_backup` set aside back where they were.
fn restore_backup(store_file: &Path, backup: &Backup) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let mut from = store_file.to_path_buf().into_os_string();
        from.push(format!(".bak-{}{suffix}", backup.stamp));
        let from = std::path::PathBuf::from(from);
        if !from.exists() {
            continue;
        }
        let mut to = store_file.to_path_buf().into_os_string();
        to.push(suffix);
        std::fs::rename(&from, std::path::PathBuf::from(to)).map_err(err)?;
    }
    Ok(())
}

/// A store file set aside by `keep_as_backup`.
struct Backup {
    stamp: u64,
}

/// The numbers of the tasks in the store's file that core's `oxtask` table
/// has no row for.
fn tasks_core_lacks(store_file: &Path, db_path: &Path) -> Result<Vec<i64>, String> {
    let store = Connection::open_with_flags(store_file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    let has_tasks: bool = store
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'task')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !has_tasks {
        return Ok(Vec::new());
    }
    let ours: Vec<i64> = store
        .prepare("SELECT n FROM task ORDER BY n")
        .map_err(err)?
        .query_map([], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let core = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    let theirs: std::collections::BTreeSet<i64> = core
        .prepare("SELECT id FROM oxtask")
        .map_err(err)?
        .query_map([], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    Ok(ours.into_iter().filter(|n| !theirs.contains(n)).collect())
}

/// Rename the store's file (and its journal files) to a backup next to it:
/// `oxtask.sqlite.bak-<unix seconds>`.
fn keep_as_backup(store_file: &Path) -> Result<Backup, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err)?
        .as_secs();
    for suffix in ["", "-wal", "-shm"] {
        let mut from = store_file.to_path_buf().into_os_string();
        from.push(suffix);
        let from = std::path::PathBuf::from(from);
        if !from.exists() {
            continue;
        }
        let mut to = store_file.to_path_buf().into_os_string();
        to.push(format!(".bak-{stamp}{suffix}"));
        std::fs::rename(&from, std::path::PathBuf::from(to)).map_err(err)?;
    }
    Ok(Backup { stamp })
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

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Invalid(e.to_string())
}

/// Read core's tables and write the fields and the entity rows.
fn main_half(tx: &Transaction<'_>, replace: bool) -> Result<Moved, DomainError> {
    // A build of bundled from before the model was renamed left its table
    // behind; links written there would never be read.
    tx.execute(
        &format!("DROP TABLE IF EXISTS {LEGACY_LINK_MODEL_TABLE}"),
        [],
    )
    .map_err(sql)?;
    if replace {
        // What an earlier run wrote: oxplow's two fields of the task model
        // and the request-link entity (nothing of another namespace's).
        tx.execute(
            "DELETE FROM model_field_value WHERE model = ?1 AND namespace = 'oxplow'",
            [MODEL],
        )
        .map_err(sql)?;
        tx.execute(
            &format!("DELETE FROM {LINK_MODEL_TABLE} WHERE store = 'oxplow'"),
            [],
        )
        .map_err(sql)?;
    }
    // The old task commands leave the command log: their audit rows (with the
    // inverses an undo would run), and the proposals waiting on them. Core has
    // no `oxplow.task.*` any more; no history of them is kept.
    tx.execute(
        "DELETE FROM command_proposal WHERE command LIKE 'oxplow.task.%'",
        [],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE command_audit SET undone_by = NULL WHERE undone_by IN
           (SELECT id FROM command_audit WHERE command LIKE 'oxplow.task.%')",
        [],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM command_audit WHERE command LIKE 'oxplow.task.%'",
        [],
    )
    .map_err(sql)?;
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
            .prepare(
                "SELECT from_item_id, to_uri, link_type, created_at FROM oxtask_link ORDER BY id",
            )
            .map_err(sql)?;
        let rows = st
            .query_map([], |r| {
                Ok(Link {
                    task_id: format!("tsk{}", r.get::<_, i64>(0)?),
                    to_uri: r.get(1)?,
                    link_type: r.get(2)?,
                    created_at: r.get(3)?,
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

    // Request links: `command` is `edited` in bundled's model.
    let mut requests: BTreeMap<String, usize> = BTreeMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT rt.request_id, rt.task_uri, rt.basis, r.thread_id, rt.turn_id, rt.at
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
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(sql)?;
        let rows: Vec<_> = rows.collect::<Result<_, _>>().map_err(sql)?;
        for (request, task, basis, thread, turn, at) in rows {
            let request = oxplow_domain::uri::build::of(
                "oxplow/request",
                &[("id", &format!("req{request}"))],
            );
            let basis = if basis == "command" {
                "edited".to_string()
            } else {
                basis
            };
            // A row as a collector's upsert writes it (no change events): the
            // model's table, store `oxplow`, named by its key.
            let key = format!("{request}|{task}|{basis}");
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO {LINK_MODEL_TABLE}
                       (uri, store, key, request, task, basis, thread, turn_id, at, written_at)
                     VALUES (?1, 'oxplow', ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))"
                ),
                params![
                    oxplow_domain::uri::build::of(
                        LINK_RESOURCE,
                        &[("store", "oxplow"), ("id", &key)]
                    ),
                    key,
                    request,
                    task,
                    basis,
                    format!("oxplow:oxplow/thread?id=thr{thread}"),
                    turn,
                    at
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
            "INSERT INTO task_link (task_id, to_uri, type, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![l.task_id, l.to_uri, l.link_type, l.created_at],
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
        std::fs::write(
            dir.path().join(".oxplow/personal.yaml"),
            "enableExtensions: [oxtask]\n",
        )
        .unwrap();
        // What bundled's first boot makes (the test project loads no
        // extensions): the model's table and the entity's.
        let c = main_db(dir.path());
        c.execute_batch(&format!(
            "CREATE TABLE {MODEL_TABLE} (uri TEXT PRIMARY KEY);
             CREATE TABLE {LEGACY_LINK_MODEL_TABLE} (uri TEXT PRIMARY KEY);
             CREATE TABLE {LINK_MODEL_TABLE} (uri TEXT PRIMARY KEY, store TEXT NOT NULL, key TEXT, request TEXT, task TEXT, basis TEXT, thread TEXT, turn_id INTEGER, at TEXT, written_at TEXT NOT NULL, removed_at TEXT, refs TEXT);"
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
               (11, '{u3}', 'command', 5, 'r1'),
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
        let said = move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        assert!(said.contains("5 tasks (1 soft-deleted), 1 links, 4 notes, 2 assignments (1 with an assignee), 5 request links"), "{said}");
        let legacy: bool = main_db(dir.path())
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?1)",
                [LEGACY_LINK_MODEL_TABLE],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!legacy, "the table of the renamed model is dropped");
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
        let link: (String, String, String, String) = s
            .query_row(
                "SELECT task_id, to_uri, type, created_at FROM task_link",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        // The link keeps the time core made it.
        assert_eq!(
            link,
            ("tsk3".into(), task_uri(2), "blocks".into(), "l".into())
        );
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
        type Link = (String, String, String, String, String, Option<i64>, String);
        let entity: Vec<Link> = m
            .prepare(&format!(
                "SELECT key, request, task, basis, thread, turn_id, at FROM {LINK_MODEL_TABLE}
                  WHERE store = 'oxplow' ORDER BY key"
            ))
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let req =
            |n: i64| oxplow_domain::uri::build::of("oxplow/request", &[("id", &format!("req{n}"))]);
        assert_eq!(entity.len(), 5);
        // `command` is `edited`; the thread is a URI; core's turn and time
        // are kept.
        assert!(entity.contains(&(
            format!("{}|{}|edited", req(11), task_uri(3)),
            req(11),
            task_uri(3),
            "edited".into(),
            "oxplow:oxplow/thread?id=thr7".into(),
            Some(5),
            "r1".into()
        )));
        assert!(entity.iter().any(|e| e.3 == "assigned" && e.1 == req(12)));
        assert!(entity
            .iter()
            .any(|e| e.3 == "transition" && e.2 == task_uri(2) && e.5.is_none()));
        // A row's URI names the model's resource and its key.
        let uri: String = m
            .query_row(
                &format!("SELECT uri FROM {LINK_MODEL_TABLE} WHERE basis = 'transition'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(uri.starts_with("oxplow:oxplow/request_task?"), "{uri}");

        // A second run is refused (the store has the three and the new one).
        let e = move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap_err();
        assert!(e.contains("already has 6 tasks"), "{e}");
    }

    #[tokio::test]
    async fn a_dry_run_keeps_nothing() {
        let dir = project().await;
        seed(dir.path());
        let said = move_to_oxtask(dir.path(), true, false, false)
            .await
            .unwrap();
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
            .query_row(
                &format!("SELECT count(*) FROM {LINK_MODEL_TABLE}"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        // The real run still works after it.
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
    }

    /// The old task commands leave the command log with the move: their audit
    /// rows and the proposals waiting on them; other commands stay.
    #[tokio::test]
    async fn the_move_deletes_the_old_task_commands_from_the_log() {
        let dir = project().await;
        seed(dir.path());
        let m = main_db(dir.path());
        m.execute_batch(
            "INSERT INTO command_audit (id, at, command, actor_kind, input_json, outcome, inverse_json)
               VALUES (901, 't', 'oxplow.task.create', 'human', '{}', 'ok', '{}'),
                      (902, 't', 'oxplow.task.transition', 'human', '{}', 'ok', NULL),
                      (903, 't', 'oxplow.thread.rename', 'human', '{}', 'ok', NULL);
             UPDATE command_audit SET undone_by = 902 WHERE id = 901;
             INSERT INTO command_proposal (created_at, command, input_json, actor_kind, key, preview_json, decision)
               VALUES ('t', 'oxplow.task.delete', '{}', 'agent', 'k1', '{}', 'pending'),
                      ('t', 'oxplow.thread.close', '{}', 'agent', 'k2', '{}', 'pending');",
        )
        .unwrap();
        drop(m);
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        let m = main_db(dir.path());
        let commands = |table: &str| -> Vec<String> {
            let mut st = m
                .prepare(&format!(
                    "SELECT command FROM {table} WHERE command LIKE 'oxplow.%' ORDER BY command"
                ))
                .unwrap();
            st.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(commands("command_audit"), vec!["oxplow.thread.rename"]);
        assert_eq!(commands("command_proposal"), vec!["oxplow.thread.close"]);
    }

    #[tokio::test]
    async fn the_move_refuses_while_oxtask_is_off() {
        let dir = project().await;
        seed(dir.path());
        std::fs::write(dir.path().join(".oxplow/personal.yaml"), "").unwrap();
        let e = move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap_err();
        assert!(e.contains("oxtask extension is off"), "{e}");
    }

    /// A copy of the main database is made before the move deletes anything
    /// from it.
    #[tokio::test]
    async fn the_move_copies_the_main_database_first() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        let state = oxplow_app::AppLayout::for_project(dir.path()).state_db_path;
        let prefix = format!("{}.pre-move-", state.file_name().unwrap().to_string_lossy());
        let copies: Vec<_> = std::fs::read_dir(state.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        let copy = Connection::open(copies[0].path()).unwrap();
        let n: i64 = copy
            .query_row("SELECT count(*) FROM oxtask", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5, "the copy still has the old tables' rows");
    }

    /// A `--replace` that fails puts the old store back, with its tasks.
    #[tokio::test]
    async fn a_failed_replace_puts_the_old_store_back() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        // A parent no task has: the store refuses it at commit.
        main_db(dir.path())
            .execute("UPDATE oxtask SET parent_id = 99 WHERE id = 6", [])
            .unwrap();
        let e = move_to_oxtask(dir.path(), false, true, false)
            .await
            .unwrap_err();
        assert!(!e.is_empty());
        let n: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5, "the first run's store is back");
    }

    #[tokio::test]
    async fn a_project_that_never_booted_the_new_oxplow_is_refused() {
        let dir = project().await;
        main_db(dir.path())
            .execute_batch(&format!("DROP TABLE {LINK_MODEL_TABLE}"))
            .unwrap();
        let e = move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap_err();
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
            let e = move_to_oxtask(dir.path(), dry_run, false, false)
                .await
                .unwrap_err();
            assert!(e.contains("oxplow is running"), "{e}");
        }
    }

    #[tokio::test]
    async fn replace_redoes_the_move_over_what_a_first_run_left() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        // Something the old run left that the new one no longer makes, and
        // another namespace's field that is not the move's to touch.
        let m = main_db(dir.path());
        m.execute_batch(&format!(
            "INSERT INTO model_field_value (model, uri, namespace, name, value) VALUES
               ('oxplow/task_record', '{stale}', 'oxplow', 'assignee', '\"x\"'),
               ('oxplow/task_record', '{kept}', 'oxtask', 'priority', '\"high\"');
             INSERT INTO {LINK_MODEL_TABLE} (uri, store, key, request, task, basis, thread, written_at) VALUES ('u|stale', 'oxplow', 'stale|k', 'r', 't', 'edited', 'x', 'w'), ('u|other', 'elsewhere', 'other|k', 'r', 't', 'edited', 'x', 'w');",
            stale = task_uri(99),
            kept = task_uri(2),
        ))
        .unwrap();
        // Without --replace the second run is still refused.
        let e = move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap_err();
        assert!(e.contains("already has"), "{e}");
        // A dry run of it changes nothing on disk.
        let said = move_to_oxtask(dir.path(), true, true, false).await.unwrap();
        assert!(said.starts_with("dry run, nothing kept: 5 tasks"), "{said}");
        let have: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(have, 5);
        let said = move_to_oxtask(dir.path(), false, true, false)
            .await
            .unwrap();
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
        assert_eq!(
            count(&format!(
                "SELECT count(*) FROM {LINK_MODEL_TABLE} WHERE store = 'oxplow'"
            )),
            5
        );
        // Another store's rows are not the move's to clear.
        assert_eq!(
            count(&format!(
                "SELECT count(*) FROM {LINK_MODEL_TABLE} WHERE store = 'elsewhere'"
            )),
            1
        );
    }

    /// `--replace` keeps what it replaces: the old store file is renamed to
    /// a backup, not deleted.
    #[tokio::test]
    async fn replace_keeps_the_old_store_as_a_backup() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        move_to_oxtask(dir.path(), false, true, false)
            .await
            .unwrap();
        let backups: Vec<_> = std::fs::read_dir(dir.path().join(".oxplow/extensions-data"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                n.starts_with("oxtask.sqlite.bak-") && !n.ends_with("-wal") && !n.ends_with("-shm")
            })
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        let old =
            Connection::open(dir.path().join(".oxplow/extensions-data").join(&backups[0])).unwrap();
        let n: i64 = old
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5);
    }

    /// A store task core's tables lack is new work: `--replace` refuses to
    /// lose it unless `--force-discard` says so.
    #[tokio::test]
    async fn replace_refuses_when_the_store_has_tasks_core_lacks() {
        let dir = project().await;
        seed(dir.path());
        move_to_oxtask(dir.path(), false, false, false)
            .await
            .unwrap();
        // The seed's highest number is 9, so tsk10 is new work.
        store(dir.path())
            .execute("INSERT INTO task (title) VALUES ('made since')", [])
            .unwrap();
        let e = move_to_oxtask(dir.path(), false, true, false)
            .await
            .unwrap_err();
        assert!(e.contains("tsk10") && e.contains("--force-discard"), "{e}");
        // Nothing changed: the store is as it was, no backup made.
        let have: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(have, 6);
        let e = move_to_oxtask(dir.path(), true, true, false)
            .await
            .unwrap_err();
        assert!(e.contains("--force-discard"), "a dry run says so too: {e}");
        // Forced: it goes, into the backup.
        move_to_oxtask(dir.path(), false, true, true).await.unwrap();
        let have: i64 = store(dir.path())
            .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(have, 5);
    }
}
