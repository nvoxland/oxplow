//! oxplow's answers to the work-item verbs (`.context/work-items.md`):
//! each runs in the caller's transaction over the task tables and answers
//! with its result — the item's `{ ref }`, as every list's — and, when it
//! can be undone, the verb that undoes it.
//! The inputs are the interface's (`oxplow_domain::work_items`), their
//! refs already canonical and already this list's — core resolves loose
//! ids and refuses another list's items before a verb runs.

use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::ids::TaskId;
use oxplow_domain::work_items::{
    provider_of, List, WorkItemCommentInput, WorkItemCreateInput, WorkItemDeleteInput,
    WorkItemLinkInput, WorkItemMoveInput, WorkItemReorderInput, WorkItemTransitionInput,
    WorkItemUpdateInput,
};
use oxplow_domain::{Actor, CommandCall, CommandError, DomainError, ThreadId, Timestamp};

use crate::mapping::{canonical_of, native_status, oxplow_native, oxplow_status, status_str};
use crate::model::{Task, TaskActorKind, TaskAuthor, TaskLinkType, TaskPriority, TaskStatus};
use crate::refs::{task_of_work_item_ref, work_item_ref, PROVIDER};
use crate::store::{self, Placement};

/// What a verb did: its result, the verb (with its input) that undoes it
/// when one does, and the tasks it changed — each answered with its
/// record ([`crate::record::record_tx`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub result: Value,
    pub inverse: Option<CommandCall>,
    pub changed: Vec<TaskId>,
}

fn invalid_at(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

/// The task `item_ref` names: `Invalid` at `field` when it is another
/// list's or names no task.
pub fn task_ref(item_ref: &str, field: &str) -> Result<TaskId, CommandError> {
    if let Ok(provider) = provider_of(item_ref) {
        if provider != PROVIDER {
            return Err(invalid_at(
                field,
                format!("`{item_ref}` is {provider}'s; this command places oxplow's own tasks"),
            ));
        }
    }
    task_of_work_item_ref(item_ref).ok_or_else(|| {
        invalid_at(
            field,
            format!("`{item_ref}` names no oxplow task (work_item:oxplow:tsk<n>)"),
        )
    })
}

/// `task` is a live task: a deleted one takes no comments or links.
fn live_task_tx(
    conn: &rusqlite::Connection,
    task: TaskId,
    item_ref: &str,
    field: &str,
) -> Result<(), CommandError> {
    let live: Option<bool> = conn
        .query_row(
            "SELECT deleted_at IS NULL FROM task WHERE id = ?1",
            [task.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| CommandError::Failed {
            message: e.to_string(),
        })?;
    match live {
        Some(true) => Ok(()),
        Some(false) => Err(invalid_at(field, format!("`{item_ref}` was deleted"))),
        None => Err(invalid_at(field, format!("no work item `{item_ref}`"))),
    }
}

fn parse_thread(raw: &str, field: &str) -> Result<ThreadId, CommandError> {
    raw.parse::<ThreadId>()
        .map_err(|e| invalid_at(field, format!("{e}")))
}

/// Who authored a task an actor files: a person (`user`), an agent
/// (`agent`, a lens acting for one included), or neither — an effect or
/// oxplow itself: the creating actor is on the run's audit and its
/// `work_item.created`, and the task isn't shown as the person's.
fn task_author(actor: &Actor) -> Option<TaskAuthor> {
    match actor {
        Actor::Human => Some(TaskAuthor::User),
        Actor::Agent { .. } => Some(TaskAuthor::Agent),
        Actor::Lens { on_behalf_of, .. } => task_author(on_behalf_of),
        Actor::Effect { .. } | Actor::System => None,
    }
}

/// Who a comment's `task_note.author` names, as [`task_author`] does for
/// a task: `user` (a person), `agent` (a lens acting for one included),
/// `effect:<extension>/<id>` or `oxplow` — never the person's for what an
/// effect or oxplow wrote.
fn note_author(actor: &Actor) -> String {
    match actor {
        Actor::Human => "user".into(),
        Actor::Agent { .. } => "agent".into(),
        Actor::Lens { on_behalf_of, .. } => note_author(on_behalf_of),
        Actor::Effect { effect } => format!("effect:{effect}"),
        Actor::System => "oxplow".into(),
    }
}

/// `create`: the row, at the end of the list it's filed on (`thread`,
/// as core resolved it; none is the backlog). An agent's task is authored
/// `agent`. The result names it: `{ ref }`.
pub fn create_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    input: WorkItemCreateInput,
) -> Result<Answer, CommandError> {
    let parent_id = input
        .parent_ref
        .as_deref()
        .map(|r| task_ref(r, "/parent_ref"))
        .transpose()?;
    let native = oxplow_native(input.native.as_ref())?;
    let thread = input
        .thread
        .as_deref()
        .map(|raw| parse_thread(raw, "/thread"))
        .transpose()?;
    let now = Timestamp::now();
    let status =
        oxplow_status(input.state, input.native_state.as_deref())?.unwrap_or(TaskStatus::Ready);
    let item = Task {
        id: TaskId::placeholder(),
        thread_id: thread,
        parent_id,
        title: input.title,
        description: input.body.unwrap_or_default(),
        status,
        priority: native.priority.unwrap_or(TaskPriority::Medium),
        sort_index: store::next_sort_index_tx(conn, thread).map_err(CommandError::from)?,
        created_by: TaskActorKind::User,
        created_at: now,
        updated_at: now,
        completed_at: (status == TaskStatus::Done).then_some(now),
        deleted_at: None,
        note_count: 0,
        author: task_author(actor),
    };
    let id = store::insert_tx(conn, &item).map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "ref": work_item_ref(id) }),
        inverse: None,
        changed: vec![id],
    })
}

/// `update`: the fields, then the status move. The inverse restores
/// exactly what was given.
pub fn update_tx(
    conn: &rusqlite::Connection,
    input: WorkItemUpdateInput,
) -> Result<Answer, CommandError> {
    let id = task_ref(&input.item_ref, "/ref")?;
    let parent = match input.parent_ref.as_deref() {
        None => None,
        Some("") => Some(None),
        Some(r) => Some(Some(task_ref(r, "/parent_ref")?)),
    };
    let native = oxplow_native(input.native.as_ref())?;
    let status = oxplow_status(input.state, input.native_state.as_deref())?;
    let not_found = |e: DomainError| match e {
        DomainError::NotFound => CommandError::Failed {
            message: format!("task {id} not found"),
        },
        other => CommandError::from(other),
    };
    let before = store::get_task_tx(conn, id)
        .map_err(not_found)?
        .ok_or_else(|| not_found(DomainError::NotFound))?;
    let now = Timestamp::now();
    let mut item = before.clone();
    if let Some(t) = &input.title {
        item.title = t.clone();
    }
    if let Some(b) = &input.body {
        item.description = b.clone();
    }
    if let Some(p) = native.priority {
        item.priority = p;
    }
    if let Some(p) = parent {
        item.parent_id = p;
    }
    item.updated_at = now;
    store::update_with_status_tx(conn, &item, status, now).map_err(not_found)?;
    let inverse = WorkItemUpdateInput {
        item_ref: input.item_ref.clone(),
        title: input.title.as_ref().map(|_| before.title.clone()),
        body: input.body.as_ref().map(|_| before.description.clone()),
        parent_ref: input
            .parent_ref
            .as_ref()
            .map(|_| before.parent_id.map(work_item_ref).unwrap_or_default()),
        state: input.state.map(|_| canonical_of(&before)),
        native_state: input.native_state.map(|_| status_str(before.status)),
        native: native
            .priority
            .map(|_| json!({ "priority": before.priority })),
    };
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: Some(CommandCall {
            name: "update".into(),
            input: serde_json::to_value(inverse).expect("input serializes"),
        }),
        changed: vec![id],
    })
}

/// `transition`: the status move. An archive keeps whether the task was
/// completed; archiving it as `done` (or `canceled`) when it isn't (or
/// is) passes through that state first, so the item reads as asked.
pub fn transition_tx(
    conn: &rusqlite::Connection,
    input: WorkItemTransitionInput,
) -> Result<Answer, CommandError> {
    let id = task_ref(&input.item_ref, "/ref")?;
    let to = oxplow_status(Some(input.to), input.native_state.as_deref())?
        .ok_or_else(|| invalid_at("/to", "a transition needs a state".into()))?;
    let now = Timestamp::now();
    let set = |status: TaskStatus| {
        store::set_status_tx(conn, id, status, now).map_err(|e| match e {
            DomainError::NotFound => CommandError::Failed {
                message: format!("task {id} not found"),
            },
            other => CommandError::from(other),
        })
    };
    let through = (to == TaskStatus::Archived)
        .then(|| native_status(input.to))
        .filter(|&status| {
            let completed = store::get_task_tx(conn, id)
                .ok()
                .flatten()
                .is_some_and(|t| t.completed_at.is_some());
            completed != (status == TaskStatus::Done)
        });
    let before = match through {
        Some(status) => Some(set(status)?.before),
        None => None,
    };
    let mut change = set(to)?;
    if let Some(before) = before {
        change.before = before;
    }
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: Some(CommandCall {
            name: "transition".into(),
            input: serde_json::to_value(WorkItemTransitionInput {
                item_ref: input.item_ref,
                to: canonical_of(&change.before),
                native_state: Some(status_str(change.before.status)),
            })
            .expect("input serializes"),
        }),
        changed: vec![id],
    })
}

/// `link`: a typed link between two live tasks. It belongs to a thread:
/// the caller's, else the linked task's, else the target's.
pub fn link_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    input: WorkItemLinkInput,
) -> Result<Answer, CommandError> {
    let from = task_ref(&input.item_ref, "/ref")?;
    let to = task_ref(&input.target, "/target")?;
    live_task_tx(conn, from, &input.item_ref, "/ref")?;
    live_task_tx(conn, to, &input.target, "/target")?;
    let link_type: TaskLinkType = serde_json::from_value(Value::String(input.link_type.clone()))
        .map_err(|_| {
            invalid_at(
                "/link_type",
                format!(
                    "`{}` isn't an oxplow link type (blocks, relates_to, discovered_from, \
                     duplicates, supersedes, replies_to)",
                    input.link_type
                ),
            )
        })?;
    let thread = match actor.thread_id() {
        Some(t) => t,
        None => {
            let on = |id: TaskId| -> Result<Option<ThreadId>, CommandError> {
                Ok(store::get_task_tx(conn, id)
                    .map_err(CommandError::from)?
                    .and_then(|t| t.thread_id))
            };
            on(from)?.or(on(to)?).ok_or_else(|| {
                invalid_at(
                    "/ref",
                    "a link between two backlog tasks needs a thread: run it from one".into(),
                )
            })?
        }
    };
    crate::satellite::create_link_tx(conn, thread, from, to, link_type)
        .map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: None,
        changed: vec![from],
    })
}

/// `comment`: a note on a live task, authored by the actor's kind. The
/// result names it: `{ ref, comment }`, the list's own id for it.
pub fn comment_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    input: WorkItemCommentInput,
) -> Result<Answer, CommandError> {
    let task = task_ref(&input.item_ref, "/ref")?;
    live_task_tx(conn, task, &input.item_ref, "/ref")?;
    if input.body.trim().is_empty() {
        return Err(invalid_at("/body", "a comment needs a body".into()));
    }
    let note = crate::satellite::add_task_note_tx(conn, task, &input.body, &note_author(actor))
        .map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "ref": input.item_ref, "comment": note.id.to_string() }),
        inverse: None,
        changed: vec![task],
    })
}

/// `delete`: the task marked deleted (its row stays).
pub fn delete_tx(
    conn: &rusqlite::Connection,
    input: WorkItemDeleteInput,
) -> Result<Answer, CommandError> {
    let id = task_ref(&input.item_ref, "/ref")?;
    store::soft_delete_tx(conn, id, Timestamp::now()).map_err(|e| match e {
        DomainError::NotFound => invalid_at("/ref", format!("no work item `{}`", input.item_ref)),
        other => CommandError::from(other),
    })?;
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: None,
        changed: vec![id],
    })
}

/// The place `before` / `after` name (at most one).
fn placement(before: &Option<String>, after: &Option<String>) -> Result<Placement, CommandError> {
    match (before, after) {
        (Some(_), Some(_)) => Err(invalid_at(
            "/after",
            "give `before` or `after`, not both".into(),
        )),
        (Some(b), None) => Ok(Placement::Before(task_ref(b, "/before")?)),
        (None, Some(a)) => Ok(Placement::After(task_ref(a, "/after")?)),
        (None, None) => Ok(Placement::End),
    }
}

/// `before` / `after` for a place (the inverse's input).
fn neighbour(place: Placement) -> (Option<String>, Option<String>) {
    match place {
        Placement::End => (None, None),
        Placement::Before(t) => (Some(work_item_ref(t)), None),
        Placement::After(t) => (None, Some(work_item_ref(t))),
    }
}

/// The list `to` names: a thread's, or the backlog (`None`).
fn move_dest(to: &List) -> Result<Option<ThreadId>, CommandError> {
    match to {
        List::Backlog => Ok(None),
        List::Thread(raw) => parse_thread(raw, "/to/thread").map(Some),
    }
}

/// The `to` naming a list: a thread's, or the backlog.
fn move_to(thread: Option<ThreadId>) -> List {
    thread.map_or(List::Backlog, |t| List::Thread(t.to_string()))
}

/// Place the task in `dest`'s list, refusing a thread that doesn't exist.
fn place(
    conn: &rusqlite::Connection,
    id: TaskId,
    dest: Option<ThreadId>,
    at: Placement,
) -> Result<store::Placed, CommandError> {
    if let Some(thread) = dest {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM threads WHERE id = ?1",
                [thread.value()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
        if exists.is_none() {
            return Err(invalid_at("/to", format!("no thread `{thread}`")));
        }
    }
    store::place_task_tx(conn, id, dest, at, Timestamp::now()).map_err(|e| match e {
        DomainError::NotFound => invalid_at("/ref", format!("no work item {}", work_item_ref(id))),
        DomainError::Invalid(message) => CommandError::Invalid {
            field: None,
            message,
        },
        other => CommandError::from(other),
    })
}

/// `reorder`: before or after another task on its own list (neither: its
/// end).
pub fn reorder_tx(
    conn: &rusqlite::Connection,
    input: WorkItemReorderInput,
) -> Result<Answer, CommandError> {
    let id = task_ref(&input.item_ref, "/ref")?;
    let at = placement(&input.before, &input.after)?;
    let Some(list) = store::get_task_tx(conn, id)
        .map_err(CommandError::from)?
        .map(|t| t.thread_id)
    else {
        return Err(invalid_at(
            "/ref",
            format!("no work item `{}`", input.item_ref),
        ));
    };
    let placed = place(conn, id, list, at)?;
    let (before, after) = neighbour(placed.from_place);
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: Some(CommandCall {
            name: "reorder".into(),
            input: serde_json::to_value(WorkItemReorderInput {
                item_ref: input.item_ref,
                before,
                after,
            })
            .expect("input serializes"),
        }),
        changed: std::iter::once(id).chain(placed.renumbered).collect(),
    })
}

/// `move`: to a thread's list or the backlog — its end, or next to a task
/// there.
pub fn move_tx(
    conn: &rusqlite::Connection,
    input: WorkItemMoveInput,
) -> Result<Answer, CommandError> {
    let id = task_ref(&input.item_ref, "/ref")?;
    let at = placement(&input.before, &input.after)?;
    let placed = place(conn, id, move_dest(&input.to)?, at)?;
    let (before, after) = neighbour(placed.from_place);
    Ok(Answer {
        result: json!({ "ref": input.item_ref }),
        inverse: Some(CommandCall {
            name: "move".into(),
            input: serde_json::to_value(WorkItemMoveInput {
                item_ref: input.item_ref,
                to: move_to(placed.from_thread),
                before,
                after,
            })
            .expect("input serializes"),
        }),
        changed: std::iter::once(id).chain(placed.renumbered).collect(),
    })
}
