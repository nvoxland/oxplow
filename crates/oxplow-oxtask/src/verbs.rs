//! oxplow's answers to the task verbs (`.context/tasks.md`):
//! each runs in the caller's transaction over the task tables and answers
//! with its result — the item's `{ ref }`, as every list's — and, when it
//! can be undone, the verb that undoes it.
//! The inputs are the interface's (`oxplow_domain::tasks`), their
//! refs already canonical and already this list's — core resolves loose
//! ids and refuses another list's items before a verb runs.

use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::ids::OxtaskId;
use oxplow_domain::tasks::{
    provider_of, TaskCommentInput, TaskCreateInput, TaskDeleteInput, TaskLinkInput,
    TaskTransitionInput, TaskUpdateInput,
};
use oxplow_domain::{Actor, CommandCall, CommandError, DomainError, Timestamp};

use crate::mapping::{oxtask_native, status_of, status_str};
use crate::model::{
    Oxtask, OxtaskActorKind, OxtaskAuthor, OxtaskLinkType, OxtaskPriority, OxtaskStatus,
};
use crate::refs::{oxtask_of_task_ref, task_ref, PROVIDER};
use crate::store;

/// What a verb did: its result, the verb (with its input) that undoes it
/// when one does, and the tasks it changed — each answered with its
/// record ([`crate::record::record_tx`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub result: Value,
    pub inverse: Option<CommandCall>,
    pub changed: Vec<OxtaskId>,
}

fn invalid_at(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

/// The task `item_ref` names: `Invalid` at `field` when it is another
/// list's or names no task.
pub fn oxtask_ref(item_ref: &str, field: &str) -> Result<OxtaskId, CommandError> {
    if let Ok(provider) = provider_of(item_ref) {
        if provider != PROVIDER {
            return Err(invalid_at(
                field,
                format!("`{item_ref}` is {provider}'s; this command places oxplow's own tasks"),
            ));
        }
    }
    oxtask_of_task_ref(item_ref).ok_or_else(|| {
        invalid_at(
            field,
            format!(
                "`{item_ref}` names no oxplow task (oxplow:oxplow/task?id=tsk<n>&provider=oxtask)"
            ),
        )
    })
}

/// `task` is a live task: a deleted one takes no comments or links.
fn live_task_tx(
    conn: &rusqlite::Connection,
    task: OxtaskId,
    item_ref: &str,
    field: &str,
) -> Result<(), CommandError> {
    let live: Option<bool> = conn
        .query_row(
            "SELECT deleted_at IS NULL FROM oxtask WHERE id = ?1",
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
        None => Err(invalid_at(field, format!("no task `{item_ref}`"))),
    }
}

/// Who authored a task an actor files: a person (`user`), an agent
/// (`agent`, a page acting for one included), or neither — an effect or
/// oxplow itself: the creating actor is on the run's audit and its
/// `task.created`, and the task isn't shown as the person's.
fn task_author(actor: &Actor) -> Option<OxtaskAuthor> {
    match actor {
        Actor::Human => Some(OxtaskAuthor::User),
        Actor::Agent { .. } => Some(OxtaskAuthor::Agent),
        Actor::Page { on_behalf_of, .. } | Actor::Component { on_behalf_of, .. } => {
            task_author(on_behalf_of)
        }
        Actor::Effect { .. } | Actor::Job { .. } | Actor::System => None,
    }
}

/// Who a comment's `oxtask_note.author` names, as [`task_author`] does for
/// a task: `user` (a person), `agent` (a page acting for one included),
/// `effect:<extension>/<id>` or `oxplow` — never the person's for what an
/// effect or oxplow wrote.
fn note_author(actor: &Actor) -> String {
    match actor {
        Actor::Human => "user".into(),
        Actor::Agent { .. } => "agent".into(),
        Actor::Page { on_behalf_of, .. } | Actor::Component { on_behalf_of, .. } => {
            note_author(on_behalf_of)
        }
        Actor::Effect { effect, .. } => oxplow_domain::uri::build::effect_source(effect),
        Actor::System => "oxplow".into(),
        Actor::Job { .. } => actor.source(),
    }
}

/// `create`: the row. Who it's assigned to is core's, never the list's.
/// An agent's task is authored
/// `agent`. The result names it: `{ ref }`.
pub fn create_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    input: TaskCreateInput,
) -> Result<Answer, CommandError> {
    let parent_id = input
        .parent_uri
        .as_deref()
        .map(|r| oxtask_ref(r, "/parent_uri"))
        .transpose()?;
    let native = oxtask_native(input.native.as_ref())?;
    let now = Timestamp::now();
    let status = input
        .stage
        .as_deref()
        .map(|s| status_of(s, "/stage"))
        .transpose()?
        .unwrap_or(OxtaskStatus::Ready);
    let item = Oxtask {
        id: OxtaskId::placeholder(),
        parent_id,
        title: input.title,
        description: input.body.unwrap_or_default(),
        status,
        priority: native.priority.unwrap_or(OxtaskPriority::Medium),
        created_by: OxtaskActorKind::User,
        created_at: now,
        updated_at: now,
        completed_at: (status == OxtaskStatus::Done).then_some(now),
        deleted_at: None,
        note_count: 0,
        author: task_author(actor),
    };
    let id = store::insert_tx(conn, &item).map_err(CommandError::from)?;
    // Its links, made with it.
    for (n, link) in input.links.iter().enumerate() {
        let at = |e: CommandError| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field: field.map(|f| format!("/links/{n}{f}")),
                message,
            },
            other => other,
        };
        link_target_tx(conn, &link.target).map_err(at)?;
        let link_type = link_type_of(&link.link_type).map_err(at)?;
        crate::satellite::create_link_tx(conn, id, &link.target, link_type)
            .map_err(CommandError::from)?;
    }
    Ok(Answer {
        result: json!({ "uri": task_ref(id) }),
        inverse: None,
        changed: vec![id],
    })
}

/// `update`: the fields, then the status move. The inverse restores
/// exactly what was given.
pub fn update_tx(
    conn: &rusqlite::Connection,
    input: TaskUpdateInput,
) -> Result<Answer, CommandError> {
    let id = oxtask_ref(&input.uri, "/uri")?;
    let parent = match input.parent_uri.as_deref() {
        None => None,
        Some("") => Some(None),
        Some(r) => Some(Some(oxtask_ref(r, "/parent_uri")?)),
    };
    let native = oxtask_native(input.native.as_ref())?;
    let status = input
        .stage
        .as_deref()
        .map(|s| status_of(s, "/stage"))
        .transpose()?;
    let not_found = |e: DomainError| match e {
        DomainError::NotFound => CommandError::Failed {
            message: format!("task {id} not found"),
        },
        other => CommandError::from(other),
    };
    let before = store::get_oxtask_tx(conn, id)
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
    let after_fields = store::update_with_status_tx(conn, &item, None, now).map_err(not_found)?;
    if let Some(to) = status {
        set_status(conn, &after_fields, to, now)?;
    }
    let inverse = TaskUpdateInput {
        uri: input.uri.clone(),
        title: input.title.as_ref().map(|_| before.title.clone()),
        body: input.body.as_ref().map(|_| before.description.clone()),
        parent_uri: input
            .parent_uri
            .as_ref()
            .map(|_| before.parent_id.map(task_ref).unwrap_or_default()),
        stage: status.map(|_| status_str(before.status)),
        native: native
            .priority
            .map(|_| json!({ "priority": before.priority })),
    };
    Ok(Answer {
        result: json!({ "uri": input.uri }),
        inverse: Some(CommandCall {
            name: "update".into(),
            input: serde_json::to_value(inverse).expect("input serializes"),
        }),
        changed: vec![id],
    })
}

/// Move `task` to `to` at `now`; the change reported is from the task as
/// it was.
fn set_status(
    conn: &rusqlite::Connection,
    task: &Oxtask,
    to: OxtaskStatus,
    now: Timestamp,
) -> Result<store::StatusChange, CommandError> {
    store::set_status_tx(conn, task.id, to, now).map_err(|e| match e {
        DomainError::NotFound => CommandError::Failed {
            message: format!("task {} not found", task.id),
        },
        other => CommandError::from(other),
    })
}

/// `transition`: the status move ([`set_status`]).
pub fn transition_tx(
    conn: &rusqlite::Connection,
    input: TaskTransitionInput,
) -> Result<Answer, CommandError> {
    let id = oxtask_ref(&input.uri, "/uri")?;
    let to = status_of(&input.to, "/to")?;
    let task = store::get_oxtask_tx(conn, id)
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::Failed {
            message: format!("task {id} not found"),
        })?;
    let change = set_status(conn, &task, to, Timestamp::now())?;
    Ok(Answer {
        result: json!({ "uri": input.uri }),
        inverse: Some(CommandCall {
            name: "transition".into(),
            input: serde_json::to_value(TaskTransitionInput {
                uri: input.uri,
                to: status_str(change.before.status),
            })
            .expect("input serializes"),
        }),
        changed: vec![id],
    })
}

/// What a link points at: an oxplow task, which must be live (`Some`),
/// or any other ref, which may dangle (`None`). Another list's item is
/// refused; whether a ref's kind is registered is core's to check.
fn link_target_tx(
    conn: &rusqlite::Connection,
    target: &str,
) -> Result<Option<OxtaskId>, CommandError> {
    match oxplow_domain::uri::build::resource_of(target) {
        None => Err(invalid_at(
            "/target",
            format!("`{target}` isn't a URI (oxplow:<extension>/<resource>?…)"),
        )),
        Some(kind) if kind == oxplow_domain::uri::resources::TASK => {
            let to = oxtask_ref(target, "/target")?;
            live_task_tx(conn, to, target, "/target")?;
            Ok(Some(to))
        }
        Some(_) => Ok(None),
    }
}

fn link_type_of(raw: &str) -> Result<OxtaskLinkType, CommandError> {
    serde_json::from_value(Value::String(raw.to_string())).map_err(|_| {
        invalid_at(
            "/link_type",
            format!(
                "`{raw}` isn't an oxplow link type (blocks, relates_to, discovered_from, \
                 duplicates, supersedes, replies_to, about)"
            ),
        )
    })
}

/// `link`: a typed link from a live task to another live task, or to any
/// ref it is `about`. A link belongs to its task. Linking twice is one
/// link. Undone by `unlink`.
pub fn link_tx(conn: &rusqlite::Connection, input: TaskLinkInput) -> Result<Answer, CommandError> {
    let from = oxtask_ref(&input.uri, "/uri")?;
    live_task_tx(conn, from, &input.uri, "/uri")?;
    link_target_tx(conn, &input.target)?;
    let link_type = link_type_of(&input.link_type)?;
    let added = crate::satellite::create_link_tx(conn, from, &input.target, link_type)
        .map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "uri": input.uri, "linked": added }),
        inverse: added.then(|| CommandCall {
            name: "unlink".into(),
            input: serde_json::to_value(&input).expect("input serializes"),
        }),
        changed: vec![from],
    })
}

/// `unlink`: remove a task's link. Undone by `link`, when there was one.
pub fn unlink_tx(
    conn: &rusqlite::Connection,
    input: TaskLinkInput,
) -> Result<Answer, CommandError> {
    let from = oxtask_ref(&input.uri, "/uri")?;
    let link_type = link_type_of(&input.link_type)?;
    let removed = crate::satellite::delete_link_tx(conn, from, &input.target, link_type)
        .map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "uri": input.uri, "unlinked": removed }),
        inverse: removed.then(|| CommandCall {
            name: "link".into(),
            input: serde_json::to_value(&input).expect("input serializes"),
        }),
        changed: vec![from],
    })
}

/// `comment`: a note on a live task, authored by the actor's kind. The
/// result names it: `{ ref, comment }`, the list's own id for it.
pub fn comment_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    input: TaskCommentInput,
) -> Result<Answer, CommandError> {
    let task = oxtask_ref(&input.uri, "/uri")?;
    live_task_tx(conn, task, &input.uri, "/uri")?;
    if input.body.trim().is_empty() {
        return Err(invalid_at("/body", "a comment needs a body".into()));
    }
    let note = crate::satellite::add_oxtask_note_tx(conn, task, &input.body, &note_author(actor))
        .map_err(CommandError::from)?;
    Ok(Answer {
        result: json!({ "uri": input.uri, "comment": note.id.to_string() }),
        inverse: None,
        changed: vec![task],
    })
}

/// `delete`: the task marked deleted (its row stays).
pub fn delete_tx(
    conn: &rusqlite::Connection,
    input: TaskDeleteInput,
) -> Result<Answer, CommandError> {
    let id = oxtask_ref(&input.uri, "/uri")?;
    store::soft_delete_tx(conn, id, Timestamp::now()).map_err(|e| match e {
        DomainError::NotFound => invalid_at("/uri", format!("no task `{}`", input.uri)),
        other => CommandError::from(other),
    })?;
    Ok(Answer {
        result: json!({ "uri": input.uri }),
        inverse: None,
        changed: vec![id],
    })
}
