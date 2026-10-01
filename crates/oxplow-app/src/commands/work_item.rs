//! The `work_item.*` commands (P2.6 / P5.C2 / P7.A1): the one write
//! surface for every provider's items. Each verb is a `Dispatch` command
//! (`.context/commands.md`): the bus routes it by the item's provider —
//! the ref's segment, or for `create` the named (else active) provider —
//! to oxplow's `Tx` core in the bus's transaction, or to another
//! provider's [`ExternalVerbs`] through its process, with **one** audit
//! row `work_item.<verb>` either way. A provider the registry doesn't
//! know is refused at `/ref` naming the registered ones; a parent or
//! link target of another provider is refused at its field; a feature the
//! provider doesn't declare (`hierarchy`, `links`, `comments`, `delete`)
//! is refused before anything runs.
//!
//! The inputs are the `v_work_item` columns, one shape for every
//! provider: `title`, `body`, `parent_ref`, a canonical `state` and the
//! provider's `native_state`, and `native` for its own fields — oxplow's
//! `thread` and `priority` ([`OxplowNative`]). `reorder` and `move` stay
//! oxplow's own (`Tx`): they place an item in oxplow's lists.
//!
//! oxplow's cores: `work_item.transition` is `task_store::set_status_tx`
//! — the row, the effort open/close it implies, `work_item.transitioned`
//! and the effort's own events commit in the bus's transaction with the
//! audit row, all caused by the run's `command.executed`. The effort's
//! snapshot pin is the effort-lifecycle pump consumer's; callers that
//! need it settle the pump (`TaskService::settle_lifecycle`).

use oxplow_domain::events::schema::{
    WorkItemCommented, WorkItemCommentedV1, WorkItemLinked, WorkItemLinkedV1,
};
use oxplow_domain::refs::build::{task_of_work_item_ref, work_item_ref};
use oxplow_domain::work_items::{
    provider_of, CanonicalState, WorkItemsProvider, WorkItemsRegistry, OXPLOW, VERBS,
};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Invokers, Lifecycle, Task,
    TaskId, TaskLinkType, TaskPriority, TaskStatus, ThreadId, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use oxplow_db::task_store::EffortTransition;

use super::{Command, Dispatch, Handler, HandlerOutput, Route, TxCtx, TxHandler};

/// oxplow's status for a canonical state.
pub fn native_status(state: CanonicalState) -> TaskStatus {
    match state {
        CanonicalState::Todo => TaskStatus::Ready,
        CanonicalState::InProgress => TaskStatus::InProgress,
        CanonicalState::Blocked => TaskStatus::Blocked,
        CanonicalState::Done => TaskStatus::Done,
        CanonicalState::Canceled => TaskStatus::Canceled,
    }
}

/// The canonical state of an oxplow task: `ready` is `todo`; `archived`
/// is `done` when it was completed, else `canceled` (the same mapping the
/// `work_item` row is projected with).
pub fn canonical_of(task: &Task) -> CanonicalState {
    state_pair(task.status, task.completed_at.is_some()).0
}

/// The `state` / `native_state` pair that names oxplow `status`:
/// `archived` rides on `done` when the task was completed (`completed`),
/// else on `canceled`.
pub fn state_pair(status: TaskStatus, completed: bool) -> (CanonicalState, String) {
    let state = match status {
        TaskStatus::Ready => CanonicalState::Todo,
        TaskStatus::InProgress => CanonicalState::InProgress,
        TaskStatus::Blocked => CanonicalState::Blocked,
        TaskStatus::Done => CanonicalState::Done,
        TaskStatus::Canceled => CanonicalState::Canceled,
        TaskStatus::Archived if completed => CanonicalState::Done,
        TaskStatus::Archived => CanonicalState::Canceled,
    };
    (state, status_str(status))
}

/// oxplow's status for a `state` / `native_state` pair (either or both;
/// `None` when neither is given). A `native_state` must be an oxplow
/// status and, with a `state`, map to it — `archived` to `done` or
/// `canceled`, the rest by name — else `Invalid` at `/native_state`.
pub fn oxplow_status(
    state: Option<CanonicalState>,
    native_state: Option<&str>,
) -> Result<Option<TaskStatus>, CommandError> {
    let invalid = |message: String| CommandError::Invalid {
        field: Some("/native_state".into()),
        message,
    };
    match (state, native_state) {
        (None, None) => Ok(None),
        (Some(state), None) => Ok(Some(native_status(state))),
        (state, Some(raw)) => {
            let status: TaskStatus =
                serde_json::from_value(Value::String(raw.into())).map_err(|_| {
                    invalid(format!(
                        "`{raw}` isn't an oxplow status (ready, in_progress, blocked, done, \
                         canceled, archived)"
                    ))
                })?;
            if let Some(state) = state {
                let fits = match status {
                    TaskStatus::Archived => {
                        matches!(state, CanonicalState::Done | CanonicalState::Canceled)
                    }
                    other => native_status(state) == other,
                };
                if !fits {
                    return Err(invalid(format!(
                        "`{raw}` isn't an oxplow status for `{}`",
                        state.as_str()
                    )));
                }
            }
            Ok(Some(status))
        }
    }
}

/// oxplow's own fields, under `native`: the thread a new task is filed
/// on (absent: the backlog) and its priority.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OxplowNative {
    /// `thr3`; only on `create` (a task moves lists with `work_item.move`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriority>,
}

fn oxplow_native(native: Option<&Value>) -> Result<OxplowNative, CommandError> {
    match native {
        None => Ok(OxplowNative::default()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| CommandError::Invalid {
            field: Some("/native".into()),
            message: format!("oxplow's native fields are `thread` and `priority`: {e}"),
        }),
    }
}

fn parse_thread(raw: &str, field: &str) -> Result<ThreadId, CommandError> {
    raw.parse::<ThreadId>().map_err(|e| CommandError::Invalid {
        field: Some(field.into()),
        message: format!("{e}"),
    })
}

/// The oxplow task `item_ref` names. Refused (an `Invalid` at `field`)
/// when it isn't a work-item ref, its provider isn't registered (the
/// message names the registered ones), or it is another provider's.
pub fn oxplow_task(
    registry: &WorkItemsRegistry,
    item_ref: &str,
    field: &str,
) -> Result<TaskId, CommandError> {
    let invalid = |message: String| CommandError::Invalid {
        field: Some(field.into()),
        message,
    };
    let provider = provider_of(item_ref).map_err(|e| invalid(e.to_string()))?;
    registry.get(provider).map_err(|e| invalid(e.to_string()))?;
    if provider != OXPLOW {
        return Err(invalid(format!(
            "`{item_ref}` is {provider}'s; this command places oxplow's own tasks"
        )));
    }
    task_of_work_item_ref(item_ref).ok_or_else(|| {
        invalid(format!(
            "`{item_ref}` names no oxplow task (work_item:oxplow:tsk<n>)"
        ))
    })
}

/// `task` is a live task: a deleted one takes no comments or links.
fn live_task_tx(
    conn: &rusqlite::Connection,
    task: TaskId,
    item_ref: &str,
    field: &str,
) -> Result<(), CommandError> {
    use rusqlite::OptionalExtension;
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
        Some(false) => Err(CommandError::Invalid {
            field: Some(field.into()),
            message: format!("`{item_ref}` was deleted"),
        }),
        None => Err(CommandError::Invalid {
            field: Some(field.into()),
            message: format!("no work item `{item_ref}`"),
        }),
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn invalid_at(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

// ---- routing: which provider answers, and may it ----

/// The registered provider `id`, or `Invalid` at `field` naming the
/// registered ones.
fn provider_named(
    registry: &WorkItemsRegistry,
    id: &str,
    field: &str,
) -> Result<WorkItemsProvider, CommandError> {
    registry
        .get(id)
        .map_err(|e| invalid_at(field, e.to_string()))
}

/// The provider `item_ref` belongs to, or `Invalid` at `field`.
fn provider_of_ref(
    registry: &WorkItemsRegistry,
    item_ref: &str,
    field: &str,
) -> Result<WorkItemsProvider, CommandError> {
    let id = provider_of(item_ref).map_err(|e| invalid_at(field, e.to_string()))?;
    provider_named(registry, id, field)
}

/// `other` (a parent, a link target) must be `provider`'s own item.
fn same_provider(provider: &str, other: &str, field: &str) -> Result<(), CommandError> {
    let owner = provider_of(other).map_err(|e| invalid_at(field, e.to_string()))?;
    if owner != provider {
        return Err(invalid_at(
            field,
            format!("`{other}` is {owner}'s; a {provider} item can't refer to it"),
        ));
    }
    Ok(())
}

/// The provider must declare `feature` (its `v_capability_provider`
/// flag) for this input.
fn supports(
    provider: &WorkItemsProvider,
    has: bool,
    feature: &str,
    field: &str,
) -> Result<(), CommandError> {
    if has {
        Ok(())
    } else {
        Err(invalid_at(
            field,
            format!("{} work items don't support {feature}", provider.id),
        ))
    }
}

/// Who answers a verb's input: the provider, checked for what the input
/// asks of it.
type Target = fn(&WorkItemsRegistry, &Value) -> Result<WorkItemsProvider, CommandError>;

fn create_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let input: WorkItemCreateInput = parse(input.clone())?;
    let (id, field) = match &input.provider {
        Some(p) => (p.clone(), "/provider"),
        None => (registry.active(), "/provider"),
    };
    let provider =
        provider_named(registry, &id, field).map_err(|e| match (&input.provider, e) {
            // The active provider isn't running: say so, never file elsewhere.
            (None, CommandError::Invalid { message, .. }) => invalid_at(
                field,
                format!("the active work-items provider isn't running: {message}"),
            ),
            (_, e) => e,
        })?;
    if let Some(parent) = &input.parent_ref {
        supports(
            &provider,
            provider.features.hierarchy,
            "hierarchy",
            "/parent_ref",
        )?;
        same_provider(&provider.id, parent, "/parent_ref")?;
    }
    Ok(provider)
}

fn update_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let input: WorkItemUpdateInput = parse(input.clone())?;
    let provider = provider_of_ref(registry, &input.item_ref, "/ref")?;
    if let Some(parent) = &input.parent_ref {
        supports(
            &provider,
            provider.features.hierarchy,
            "hierarchy",
            "/parent_ref",
        )?;
        if !parent.is_empty() {
            same_provider(&provider.id, parent, "/parent_ref")?;
        }
    }
    Ok(provider)
}

fn ref_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let item_ref = input["ref"].as_str().unwrap_or_default();
    provider_of_ref(registry, item_ref, "/ref")
}

fn link_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let input: WorkItemLinkInput = parse(input.clone())?;
    let provider = provider_of_ref(registry, &input.item_ref, "/ref")?;
    supports(&provider, provider.features.links, "links", "/ref")?;
    same_provider(&provider.id, &input.target, "/target")?;
    Ok(provider)
}

fn comment_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let provider = ref_target(registry, input)?;
    supports(&provider, provider.features.comments, "comments", "/ref")?;
    Ok(provider)
}

fn delete_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let provider = ref_target(registry, input)?;
    supports(&provider, provider.features.delete, "delete", "/ref")?;
    Ok(provider)
}

/// A `work_item.<verb>` command: routed by `target` to oxplow's `tx` core
/// or to the provider's process. The external run hands the provider the
/// input less `provider`, and renames its inverse (a verb) back to
/// `work_item.<verb>`, so an undo dispatches again.
fn dispatching(
    spec: CommandSpec,
    registry: WorkItemsRegistry,
    verb: &'static str,
    target: Target,
    tx: Arc<TxHandler>,
) -> Command {
    let route_registry = registry.clone();
    let route = Arc::new(move |input: &Value| {
        target(&route_registry, input).map(|p| match p.external {
            None => Route::Tx,
            Some(_) => Route::External(format!("provider `{}`", p.id)),
        })
    });
    let external = Arc::new(move |actor: Actor, input: Value| {
        let registry = registry.clone();
        Box::pin(async move {
            let provider = target(&registry, &input)?;
            let verbs = provider.external.ok_or_else(|| CommandError::Failed {
                message: format!(
                    "`{}` was routed outside the transaction, but provider `{}` runs inside it",
                    spec_name(verb),
                    provider.id
                ),
            })?;
            let mut input = input;
            if let Value::Object(fields) = &mut input {
                fields.remove("provider");
            }
            let out = verbs.invoke(&actor, verb, input).await?;
            let inverse = out
                .inverse
                .map(|c| {
                    if !VERBS.contains(&c.name.as_str()) {
                        return Err(CommandError::Failed {
                            message: format!(
                                "provider `{}` returned an inverse `{}`, which isn't a work-items \
                                 verb",
                                provider.id, c.name
                            ),
                        });
                    }
                    Ok(CommandCall {
                        name: spec_name(&c.name),
                        input: c.input,
                    })
                })
                .transpose()?;
            Ok(HandlerOutput {
                result: out.result,
                inverse,
                events: out.events,
                after_commit: None,
            })
        }) as super::ExternalFuture
    });
    Command::new(
        spec,
        Handler::Dispatch(Dispatch {
            route,
            tx,
            external,
        }),
    )
    .expect("a work_item command registers")
}

fn spec_name(verb: &str) -> String {
    format!("work_item.{verb}")
}

fn spec(
    name: &str,
    summary: &str,
    schema: Value,
    confirm: Confirm,
    undoable: bool,
    atomicity: Atomicity,
) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity,
        effect: oxplow_domain::CommandEffect::Record,
    }
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

// ---- work_item.transition ----

pub const NAME: &str = "work_item.transition";

/// Move an item to a canonical state, and optionally to one of its
/// provider's own states that maps to it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemTransitionInput {
    /// The item's ref (`work_item:oxplow:tsk42`, `work_item:linear:ENG-12`).
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub to: CanonicalState,
    /// The provider's own state, which must map to `to` (oxplow:
    /// `archived` with `done` or `canceled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
}

pub fn spec_transition() -> CommandSpec {
    spec(
        NAME,
        "Move a work item to a canonical state (todo, in_progress, blocked, done, canceled), \
         optionally naming the provider's own state; for oxplow's tasks entering or leaving \
         in_progress opens or closes the effort.",
        schema_of::<WorkItemTransitionInput>(),
        Confirm::Never,
        true,
        Atomicity::Dispatch,
    )
}

/// oxplow's core: `set_status_tx`. The handler is pure: it runs inside a
/// transaction the bus may retry.
fn tx_transition(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemTransitionInput = parse(input)?;
        let id = oxplow_task(&registry, &input.item_ref, "/ref")?;
        let to = oxplow_status(Some(input.to), input.native_state.as_deref())?
            .expect("a state was given");
        let now = Timestamp::now();
        let set = |status: TaskStatus| {
            oxplow_db::task_store::set_status_tx(ctx.conn, &ctx.events, id, status, now).map_err(
                |e| match e {
                    oxplow_domain::DomainError::NotFound => CommandError::Failed {
                        message: format!("task {id} not found"),
                    },
                    other => CommandError::from(other),
                },
            )
        };
        // An archive keeps whether the task was completed; archiving it
        // as `done` (or `canceled`) when it isn't (or is) passes through
        // that state first, so the item reads as asked.
        let through = (to == TaskStatus::Archived)
            .then(|| native_status(input.to))
            .filter(|&status| {
                let completed = oxplow_db::task_store::get_task_tx(ctx.conn, id)
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
        ctx.claim(
            matches!(change.effort, EffortTransition::Opened(_)),
            &format!("moving {id} to in_progress"),
        )?;
        Ok(HandlerOutput {
            result: serde_json::to_value(&change.after).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: NAME.into(),
                input: serde_json::to_value(WorkItemTransitionInput {
                    item_ref: input.item_ref,
                    to: canonical_of(&change.before),
                    native_state: Some(status_str(change.before.status)),
                })
                .expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    })
}

fn status_str(status: TaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("a status serializes as a string")
}

pub fn command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        spec_transition(),
        registry.clone(),
        "transition",
        ref_target,
        tx_transition(registry),
    )
}

// ---- work_item.create ----

pub const CREATE: &str = "work_item.create";

/// A new item on a provider (the active one when none is named),
/// optionally straight into a state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCreateInput {
    /// The provider to file on (`oxplow`, `linear`); absent, the active
    /// one — which must be running (never a silent fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's ref, on the same provider (needs `hierarchy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    /// `todo` when absent; `in_progress` on oxplow opens the effort in
    /// the same run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CanonicalState>,
    /// The provider's own state, which must map to `state` when both are
    /// given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
    /// The provider's own fields (oxplow: `{ thread?, priority? }`;
    /// absent `thread` files onto the backlog).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}

pub fn create_spec() -> CommandSpec {
    spec(
        CREATE,
        "File a work item on a provider (the active one by default), optionally straight into \
         a state. oxplow's native fields: { thread, priority }; absent thread files onto the \
         backlog, in_progress opens its effort in the same transaction.",
        schema_of::<WorkItemCreateInput>(),
        Confirm::Never,
        // Undoing a filing would be deleting an item — not what undo is for.
        false,
        Atomicity::Dispatch,
    )
}

/// oxplow's core: `insert_logged_tx` — the row (at the end of its list),
/// `work_item.created`, and the effort when filed `in_progress`, caused
/// by the run. An agent's task is authored `agent`.
fn tx_create(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemCreateInput = parse(input)?;
        let parent_id = input
            .parent_ref
            .as_deref()
            .map(|r| oxplow_task(&registry, r, "/parent_ref"))
            .transpose()?;
        let native = oxplow_native(input.native.as_ref())?;
        let thread = native
            .thread
            .as_deref()
            .map(|raw| parse_thread(raw, "/native/thread"))
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
            sort_index: oxplow_db::task_store::next_sort_index_tx(ctx.conn, thread)
                .map_err(CommandError::from)?,
            created_by: oxplow_domain::TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: (status == TaskStatus::Done).then_some(now),
            deleted_at: None,
            note_count: 0,
            author: Some(if ctx.actor.is_agent_driven() {
                oxplow_domain::TaskAuthor::Agent
            } else {
                oxplow_domain::TaskAuthor::User
            }),
        };
        let (id, effort) = oxplow_db::task_store::insert_logged_tx(ctx.conn, &ctx.events, &item)
            .map_err(CommandError::from)?;
        ctx.claim(effort.is_some(), "filing a task in_progress")?;
        let row = oxplow_db::task_store::get_task_tx(ctx.conn, id)
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::Failed {
                message: format!("task {id} vanished"),
            })?;
        let mut result = serde_json::to_value(&row).expect("Task serializes");
        result["ref"] = Value::String(work_item_ref(id));
        Ok(HandlerOutput {
            result,
            inverse: None,
            events: Vec::new(),
            after_commit: None,
        })
    })
}

pub fn create_command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        create_spec(),
        registry.clone(),
        "create",
        create_target,
        tx_create(registry),
    )
}

// ---- work_item.update ----

pub const UPDATE: &str = "work_item.update";

/// Edit an item's fields and, optionally, its state — one run. Absent
/// fields are left alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemUpdateInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The parent's ref on the same provider, or `""` to detach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CanonicalState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<String>,
    /// The provider's own fields to change (oxplow: `{ priority? }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}

pub fn update_spec() -> CommandSpec {
    spec(
        UPDATE,
        "Edit a work item's title, body, parent or native fields and, optionally, move it to \
         a state — all in one run (see work_item.transition for the state's effects).",
        schema_of::<WorkItemUpdateInput>(),
        Confirm::Never,
        true,
        Atomicity::Dispatch,
    )
}

/// oxplow's core: `update_with_status_tx` — `work_item.edited` for the
/// fields, then the status move with everything it implies, all caused
/// by the run. The inverse restores exactly what was given.
fn tx_update(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemUpdateInput = parse(input)?;
        let id = oxplow_task(&registry, &input.item_ref, "/ref")?;
        let parent = match input.parent_ref.as_deref() {
            None => None,
            Some("") => Some(None),
            Some(r) => Some(Some(oxplow_task(&registry, r, "/parent_ref")?)),
        };
        let native = oxplow_native(input.native.as_ref())?;
        if native.thread.is_some() {
            return Err(invalid_at(
                "/native/thread",
                "a task moves to another thread's list with `work_item.move`".into(),
            ));
        }
        let status = oxplow_status(input.state, input.native_state.as_deref())?;
        let not_found = |e: oxplow_domain::DomainError| match e {
            oxplow_domain::DomainError::NotFound => CommandError::Failed {
                message: format!("task {id} not found"),
            },
            other => CommandError::from(other),
        };
        let before = oxplow_db::task_store::get_task_tx(ctx.conn, id)
            .map_err(not_found)?
            .ok_or_else(|| not_found(oxplow_domain::DomainError::NotFound))?;
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
        let (after, effort) =
            oxplow_db::task_store::update_with_status_tx(ctx.conn, &ctx.events, &item, status, now)
                .map_err(not_found)?;
        ctx.claim(
            matches!(effort, EffortTransition::Opened(_)),
            &format!("moving {id} to in_progress"),
        )?;
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
        Ok(HandlerOutput {
            result: serde_json::to_value(&after).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: UPDATE.into(),
                input: serde_json::to_value(inverse).expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    })
}

pub fn update_command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        update_spec(),
        registry.clone(),
        "update",
        update_target,
        tx_update(registry),
    )
}

// ---- work_item.link ----

pub const LINK: &str = "work_item.link";

/// A typed link from one item to another of the same provider.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemLinkInput {
    /// The item linked from.
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// The item linked to (the same provider's).
    pub target: String,
    /// The provider names its own types (oxplow: blocks, relates_to,
    /// discovered_from, duplicates, supersedes, replies_to).
    pub link_type: String,
}

/// oxplow's core: `task_satellite::create_link_tx`, logging
/// `work_item.linked`. The link belongs to a thread: the caller's, else
/// the linked task's, else the target's.
fn tx_link(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemLinkInput = parse(input)?;
        let from = oxplow_task(&registry, &input.item_ref, "/ref")?;
        let to = oxplow_task(&registry, &input.target, "/target")?;
        live_task_tx(ctx.conn, from, &input.item_ref, "/ref")?;
        live_task_tx(ctx.conn, to, &input.target, "/target")?;
        let link_type: TaskLinkType =
            serde_json::from_value(Value::String(input.link_type.clone())).map_err(|_| {
                invalid_at(
                    "/link_type",
                    format!(
                        "`{}` isn't an oxplow link type (blocks, relates_to, discovered_from, \
                         duplicates, supersedes, replies_to)",
                        input.link_type
                    ),
                )
            })?;
        let thread = match ctx.actor.thread_id() {
            Some(t) => t,
            None => {
                let on = |id: TaskId| -> Result<Option<ThreadId>, CommandError> {
                    Ok(oxplow_db::task_store::get_task_tx(ctx.conn, id)
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
        let link = oxplow_db::task_satellite::create_link_tx(ctx.conn, thread, from, to, link_type)
            .map_err(CommandError::from)?;
        let event = ctx
            .events
            .typed::<WorkItemLinked>(&WorkItemLinkedV1 {
                work_item: input.item_ref.clone(),
                target: input.target.clone(),
                link_type: input.link_type.clone(),
            })
            .with_subject([input.item_ref.clone(), input.target.clone()]);
        Ok(HandlerOutput {
            result: serde_json::to_value(&link).expect("TaskLink serializes"),
            inverse: None,
            events: vec![event],
            after_commit: None,
        })
    })
}

pub fn link_command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        spec(
            LINK,
            "Link one work item to another of the same provider, by a link type the provider \
             names (oxplow: blocks, relates_to, discovered_from, duplicates, supersedes, \
             replies_to).",
            schema_of::<WorkItemLinkInput>(),
            Confirm::Never,
            false,
            Atomicity::Dispatch,
        ),
        registry.clone(),
        "link",
        link_target,
        tx_link(registry),
    )
}

// ---- work_item.comment ----

pub const COMMENT: &str = "work_item.comment";

/// A comment on an item (oxplow: a task note).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCommentInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// Markdown.
    pub body: String,
}

/// oxplow's core: `task_satellite::add_task_note_tx`, logging
/// `work_item.commented`; the note is authored by the actor's kind.
fn tx_comment(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemCommentInput = parse(input)?;
        let task = oxplow_task(&registry, &input.item_ref, "/ref")?;
        live_task_tx(ctx.conn, task, &input.item_ref, "/ref")?;
        if input.body.trim().is_empty() {
            return Err(invalid_at("/body", "a comment needs a body".into()));
        }
        let author = if ctx.actor.is_agent_driven() {
            "agent"
        } else {
            "user"
        };
        let note = oxplow_db::task_satellite::add_task_note_tx(ctx.conn, task, &input.body, author)
            .map_err(CommandError::from)?;
        let event = ctx
            .events
            .typed::<WorkItemCommented>(&WorkItemCommentedV1 {
                work_item: input.item_ref.clone(),
                comment: format!("task_note:{}", note.id),
            })
            .with_subject([input.item_ref.clone()]);
        Ok(HandlerOutput {
            result: serde_json::to_value(&note).expect("TaskNote serializes"),
            inverse: None,
            events: vec![event],
            after_commit: None,
        })
    })
}

pub fn comment_command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        spec(
            COMMENT,
            "Comment on a work item (oxplow: a note shown with the task).",
            schema_of::<WorkItemCommentInput>(),
            Confirm::Never,
            false,
            Atomicity::Dispatch,
        ),
        registry.clone(),
        "comment",
        comment_target,
        tx_comment(registry),
    )
}

// ---- work_item.delete ----

pub const DELETE: &str = "work_item.delete";

/// Remove an item (oxplow: soft — the row stays, marked deleted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemDeleteInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
}

/// oxplow's core: `soft_delete_tx`. An open effort on it closes;
/// `work_item.deleted@1` is logged caused by the run.
fn tx_delete(registry: WorkItemsRegistry) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemDeleteInput = parse(input)?;
        let id = oxplow_task(&registry, &input.item_ref, "/ref")?;
        oxplow_db::task_store::soft_delete_tx(ctx.conn, &ctx.events, id, Timestamp::now())
            .map_err(|e| match e {
                oxplow_domain::DomainError::NotFound => {
                    invalid_at("/ref", format!("no work item `{}`", input.item_ref))
                }
                other => CommandError::from(other),
            })?;
        Ok(HandlerOutput {
            result: json!({ "ref": input.item_ref }),
            inverse: None,
            events: Vec::new(),
            after_commit: None,
        })
    })
}

/// Destructive (asks first) and not undoable; only on a provider that
/// declares `delete`.
pub fn delete_command(registry: WorkItemsRegistry) -> Command {
    dispatching(
        spec(
            DELETE,
            "Delete a work item (oxplow: its open effort closes).",
            schema_of::<WorkItemDeleteInput>(),
            Confirm::Destructive,
            false,
            Atomicity::Dispatch,
        ),
        registry.clone(),
        "delete",
        delete_target,
        tx_delete(registry),
    )
}

// ---- work_item.reorder / work_item.move: oxplow's lists ----

pub const REORDER: &str = "work_item.reorder";
pub const MOVE: &str = "work_item.move";

/// `work_item.reorder`: put an item before or after another in its own
/// list (neither: at its end).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemReorderInput {
    /// The task's ref (`work_item:oxplow:tsk42`).
    #[serde(rename = "ref")]
    pub item_ref: String,
    /// Put it just before this item of the same list.
    #[serde(default)]
    pub before: Option<String>,
    /// Put it just after this item of the same list.
    #[serde(default)]
    pub after: Option<String>,
}

/// Which list `work_item.move` takes an item to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoveTo {
    /// The project-wide backlog.
    Backlog,
    /// A thread's list (`thr3`).
    Thread(String),
}

impl MoveTo {
    fn thread(&self) -> Result<Option<ThreadId>, CommandError> {
        match self {
            MoveTo::Backlog => Ok(None),
            MoveTo::Thread(raw) => parse_thread(raw, "/to/thread").map(Some),
        }
    }

    fn of(thread: Option<ThreadId>) -> Self {
        thread.map_or(MoveTo::Backlog, |t| MoveTo::Thread(t.to_string()))
    }
}

/// `work_item.move`: take an item to another list — its end, or next to
/// an item there.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemMoveInput {
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub to: MoveTo,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

/// The place `before` / `after` name (at most one).
fn placement(
    registry: &WorkItemsRegistry,
    before: &Option<String>,
    after: &Option<String>,
) -> Result<oxplow_db::task_store::Placement, CommandError> {
    use oxplow_db::task_store::Placement;
    match (before, after) {
        (Some(_), Some(_)) => Err(invalid_at(
            "/after",
            "give `before` or `after`, not both".into(),
        )),
        (Some(b), None) => Ok(Placement::Before(oxplow_task(registry, b, "/before")?)),
        (None, Some(a)) => Ok(Placement::After(oxplow_task(registry, a, "/after")?)),
        (None, None) => Ok(Placement::End),
    }
}

/// `before` / `after` for a place (the inverse's input).
fn neighbour(place: oxplow_db::task_store::Placement) -> (Option<String>, Option<String>) {
    use oxplow_db::task_store::Placement;
    match place {
        Placement::End => (None, None),
        Placement::Before(t) => (Some(work_item_ref(t)), None),
        Placement::After(t) => (None, Some(work_item_ref(t))),
    }
}

/// Place the task in `dest`'s list, refusing a thread that doesn't exist
/// and an effort the actor may not claim.
fn place(
    ctx: &TxCtx<'_>,
    id: TaskId,
    dest: Option<ThreadId>,
    at: oxplow_db::task_store::Placement,
) -> Result<oxplow_db::task_store::Placed, CommandError> {
    if let Some(thread) = dest {
        use rusqlite::OptionalExtension;
        let exists: Option<i64> = ctx
            .conn
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
    let placed =
        oxplow_db::task_store::place_task_tx(ctx.conn, &ctx.events, id, dest, at, Timestamp::now())
            .map_err(|e| match e {
                oxplow_domain::DomainError::NotFound => {
                    invalid_at("/ref", format!("no work item {}", work_item_ref(id)))
                }
                oxplow_domain::DomainError::Invalid(message) => CommandError::Invalid {
                    field: None,
                    message,
                },
                other => CommandError::from(other),
            })?;
    ctx.claim(
        placed.from_thread != dest
            && dest.is_some()
            && placed.task.status == TaskStatus::InProgress,
        &format!("moving {id}, which is in progress, onto a thread"),
    )?;
    Ok(placed)
}

pub fn reorder_command(registry: WorkItemsRegistry) -> Command {
    let spec = spec(
        REORDER,
        "Put a task before or after another in its own list (neither: at its end).",
        schema_of::<WorkItemReorderInput>(),
        Confirm::Never,
        true,
        Atomicity::Tx,
    );
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemReorderInput = parse(input)?;
        let id = oxplow_task(&registry, &input.item_ref, "/ref")?;
        let at = placement(&registry, &input.before, &input.after)?;
        let current = oxplow_db::task_store::get_task_tx(ctx.conn, id)
            .map_err(CommandError::from)?
            .map(|t| t.thread_id);
        let Some(list) = current else {
            return Err(invalid_at(
                "/ref",
                format!("no work item `{}`", input.item_ref),
            ));
        };
        let placed = place(ctx, id, list, at)?;
        let (before, after) = neighbour(placed.from_place);
        Ok(HandlerOutput {
            result: serde_json::to_value(&placed.task).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: REORDER.into(),
                input: serde_json::to_value(WorkItemReorderInput {
                    item_ref: input.item_ref,
                    before,
                    after,
                })
                .expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(spec, handler).expect("work_item.reorder registers")
}

pub fn move_command(registry: WorkItemsRegistry) -> Command {
    let spec = spec(
        MOVE,
        "Move a task to a thread's list or the backlog (at the end, or before/after an item \
         there); an in-progress task's effort moves with it.",
        schema_of::<WorkItemMoveInput>(),
        Confirm::Never,
        true,
        Atomicity::Tx,
    );
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WorkItemMoveInput = parse(input)?;
        let id = oxplow_task(&registry, &input.item_ref, "/ref")?;
        let at = placement(&registry, &input.before, &input.after)?;
        let placed = place(ctx, id, input.to.thread()?, at)?;
        let (before, after) = neighbour(placed.from_place);
        Ok(HandlerOutput {
            result: serde_json::to_value(&placed.task).expect("Task serializes"),
            inverse: Some(CommandCall {
                name: MOVE.into(),
                input: serde_json::to_value(WorkItemMoveInput {
                    item_ref: input.item_ref,
                    to: MoveTo::of(placed.from_thread),
                    before,
                    after,
                })
                .expect("input serializes"),
            }),
            events: Vec::new(),
            after_commit: None,
        })
    }));
    Command::new(spec, handler).expect("work_item.move registers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;
    use oxplow_domain::StreamId;

    /// P5.C2: the commands take canonical refs and refuse another
    /// provider's, naming the registered ones.
    #[tokio::test]
    async fn a_foreign_ref_is_refused_naming_the_registered_providers() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for (name, input) in [
            (
                NAME,
                json!({ "ref": "work_item:linear:ENG-12", "to": "done" }),
            ),
            (
                UPDATE,
                json!({ "ref": "work_item:linear:ENG-12", "title": "x" }),
            ),
            (
                COMMENT,
                json!({ "ref": "work_item:linear:ENG-12", "body": "x" }),
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, name, input, false)
                .await
                .unwrap_err();
            match err {
                CommandError::Invalid { field, message } => {
                    assert_eq!(field.as_deref(), Some("/ref"), "{name}");
                    assert_eq!(
                        message, "no work-items provider `linear`; registered: oxplow",
                        "{name}"
                    );
                }
                other => panic!("{name}: expected Invalid, got {other:?}"),
            }
        }
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": "tsk42", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("work_item:<provider>:<id>"),
            "{err}"
        );
    }

    /// P7.A1: `transition` names a canonical state and, optionally, the
    /// provider's own — oxplow's `archived` rides on `done` or `canceled`;
    /// a native state that doesn't map to the canonical one is refused at
    /// `/native_state`; the inverse carries both, so undo restores the
    /// exact status.
    #[tokio::test]
    async fn a_transition_takes_a_state_and_an_optional_native_state() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let t = work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": t, "to": "in_progress", "native_state": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/native_state"),
            "{err:?}"
        );
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": t, "to": "done", "native_state": "Shipped" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("isn't an oxplow status"), "{err}");

        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": t, "to": "done", "native_state": "archived" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "archived");
        let inverse = out.inverse.clone().unwrap();
        assert_eq!(inverse.name, NAME);
        assert_eq!(
            inverse.input,
            json!({ "ref": t, "to": "in_progress", "native_state": "in_progress" })
        );
        let row = fx
            .svc
            .sql
            .query_sql(
                "SELECT state, native_state FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(t.clone())],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            row.rows[0],
            vec![
                oxplow_db::SqlCell::Text("done".into()),
                oxplow_db::SqlCell::Text("archived".into())
            ]
        );
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_domain::stores::TaskStore as _;
        let back = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(back.status, TaskStatus::InProgress);
    }

    /// P7.A1: the input is the `v_work_item` shape — oxplow's own fields
    /// ride under `native` and nothing else does.
    #[tokio::test]
    async fn a_create_keeps_oxplows_fields_under_native() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({
                    "title": "filed",
                    "body": "the body",
                    "native": { "thread": fx.thread.to_string(), "priority": "high" },
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["priority"], "high");
        assert_eq!(out.result["thread_id"], fx.thread.to_string());
        assert_eq!(out.result["description"], "the body");
        for (input, field) in [
            (
                json!({ "title": "x", "native": { "bogus": 1 } }),
                Some("/native"),
            ),
            (json!({ "title": "x", "priority": "high" }), None),
            (
                json!({ "title": "x", "native": { "thread": "nope" } }),
                Some("/native/thread"),
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, CREATE, input.clone(), false)
                .await
                .unwrap_err();
            match err {
                CommandError::Invalid { field: got, .. } => {
                    if let Some(want) = field {
                        assert_eq!(got.as_deref(), Some(want), "{input}");
                    }
                }
                other => panic!("{input}: {other:?}"),
            }
        }
    }

    /// A link made by a person (no thread of their own) belongs to the
    /// linked task's thread; two backlog tasks can't be linked that way.
    #[tokio::test]
    async fn a_persons_link_takes_the_tasks_thread() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let from = work_item_ref(fx.task);
        let other = file_on(&fx, "other", None).await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "ref": from, "target": other, "link_type": "blocks" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["thread_id"], fx.thread.to_string());
        let another = file_on(&fx, "another", None).await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "ref": other, "target": another, "link_type": "relates_to" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread"), "{err}");
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "ref": from, "target": other, "link_type": "caused_by" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/link_type"),
            "{err:?}"
        );
    }

    /// tsk572: a deleted task takes no comments or links.
    #[tokio::test]
    async fn a_deleted_task_takes_no_comments_or_links() {
        use oxplow_domain::stores::TaskStore as _;
        let fx = crate::test_fixtures::services_with_effort().await;
        let other = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "other" }), false)
            .await
            .unwrap()
            .result["ref"]
            .as_str()
            .unwrap()
            .to_string();
        fx.svc.task_store.soft_delete(fx.task).await.unwrap();
        let gone = work_item_ref(fx.task);
        for (name, input) in [
            (COMMENT, json!({ "ref": gone, "body": "hello" })),
            (
                LINK,
                json!({ "ref": other, "target": gone, "link_type": "blocks" }),
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(
                    &Actor::Agent {
                        thread_id: Some(fx.thread),
                        stream_id: None,
                    },
                    name,
                    input,
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { message, .. } if message.contains("deleted")),
                "{name}: {err:?}"
            );
        }
    }

    /// `work_item.link` and `work_item.comment` write the link and the
    /// note, each with its event, caused by the run.
    #[tokio::test]
    async fn links_and_comments_are_commands_with_their_events() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let from = work_item_ref(fx.task);
        let other = fx
            .svc
            .commands
            .run(&agent, CREATE, json!({ "title": "other" }), false)
            .await
            .unwrap()
            .result["ref"]
            .as_str()
            .unwrap()
            .to_string();
        let linked = fx
            .svc
            .commands
            .run(
                &agent,
                LINK,
                json!({ "ref": from, "target": other, "link_type": "blocks" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(linked.result["link_type"], "blocks");
        assert_eq!(linked.result["thread_id"], fx.thread.to_string());
        let commented = fx
            .svc
            .commands
            .run(
                &agent,
                COMMENT,
                json!({ "ref": from, "body": "see [[src/lib.rs]]" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(commented.result["author"], "agent");
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        for (out, event_type) in [
            (&linked, "work_item.linked"),
            (&commented, "work_item.commented"),
        ] {
            let caused: Vec<&str> = events
                .iter()
                .filter(|e| e.envelope.cause == out.event_id)
                .map(|e| e.envelope.event_type.as_str())
                .collect();
            assert_eq!(caused, vec![event_type]);
        }
        let err = fx
            .svc
            .commands
            .run(&agent, COMMENT, json!({ "ref": from, "body": "  " }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("body"), "{err}");
    }

    /// P2.6.3 (tsk455): a transition is one transaction with its audit —
    /// the status, the effort close, `work_item.transitioned` and
    /// `effort.closed` all carry the actor's source and are caused by the
    /// run's `command.executed`.
    #[tokio::test]
    async fn a_transition_commits_with_its_audit_and_names_its_cause() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        assert_eq!(spec_transition().atomicity, Atomicity::Dispatch);
        let outcome = fx
            .svc
            .commands
            .run(
                &agent,
                NAME,
                json!({ "ref": work_item_ref(fx.task), "to": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(outcome.result["status"], "done");
        let executed = outcome.event_id.expect("a write is recorded");

        let events = fx.svc.event_log_store.read_after(0, 50).await.unwrap();
        let caused: Vec<(&str, &str)> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| (e.envelope.event_type.as_str(), e.envelope.source.as_str()))
            .collect();
        let source = format!("agent:{}", fx.thread);
        assert_eq!(
            caused,
            vec![
                ("effort.closed", source.as_str()),
                ("work_item.transitioned", source.as_str()),
            ]
        );
        assert!(events
            .iter()
            .any(|e| e.envelope.id == executed && e.envelope.event_type == "command.executed"));
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert!(effort.ended_at.is_some());
    }

    /// `work_item.update`: fields and state commit together, audited and
    /// undoable — an undo restores both.
    #[tokio::test]
    async fn an_update_edits_fields_and_state_atomically_and_undoes() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let bus = &fx.svc.commands;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let out = bus
            .run(
                &agent,
                UPDATE,
                json!({
                    "ref": work_item_ref(fx.task), "title": "renamed", "state": "blocked",
                    "native": { "priority": "urgent" }
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["title"], "renamed");
        assert_eq!(out.result["status"], "blocked");
        assert_eq!(out.result["priority"], "urgent");
        let executed = out.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(
            caused,
            vec![
                "work_item.edited",
                "effort.closed",
                "work_item.transitioned"
            ]
        );

        bus.undo(&agent, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_domain::stores::TaskStore as _;
        let back = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(back.title, "t");
        assert_eq!(back.status, TaskStatus::InProgress);
        assert_eq!(back.priority, TaskPriority::Medium);
        let err = bus
            .run(
                &agent,
                UPDATE,
                json!({ "ref": work_item_ref(fx.task), "native": { "thread": "thr2" } }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("work_item.move"), "{err}");
    }

    /// A queued thread (tsk466) may edit a task and move it anywhere but
    /// `in_progress`: task bookkeeping isn't a claim on the worktree.
    #[tokio::test]
    async fn a_queued_thread_edits_and_finishes_tasks() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                UPDATE,
                json!({ "ref": work_item_ref(fx.task), "title": "renamed", "state": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "done");
        use oxplow_domain::stores::TaskStore as _;
        let row = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("renamed", TaskStatus::Done)
        );
    }

    /// Moving a task to `in_progress` opens an effort — a claim — which
    /// only the stream's writer thread may take. A refused run writes
    /// nothing, not the fields either, and is audited as denied.
    #[tokio::test]
    async fn a_queued_thread_cannot_claim() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let later = file_on(&fx, "later", Some(fx.thread)).await;
        for (name, input) in [
            (
                UPDATE,
                json!({ "ref": later, "title": "renamed", "state": "in_progress" }),
            ),
            (NAME, json!({ "ref": later, "to": "in_progress" })),
        ] {
            let err = fx
                .svc
                .commands
                .run(&queued, name, input, false)
                .await
                .unwrap_err();
            assert!(
                matches!(err, CommandError::Denied { .. }),
                "{name}: {err:?}"
            );
        }
        let id = task_of_work_item_ref(&later).unwrap();
        use oxplow_domain::stores::TaskStore as _;
        let row = fx.svc.task_store.get(id).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("later", TaskStatus::Ready)
        );
        let recent = fx.svc.commands.audit_store().list_recent(10).await.unwrap();
        let denied = recent
            .iter()
            .filter(|r| r.outcome == oxplow_domain::events::schema::CommandOutcome::Denied)
            .count();
        assert_eq!(denied, 2, "{recent:?}");
    }

    /// `work_item.create` (tsk463): filing a task is audited to the actor;
    /// filed straight into `in_progress` it opens the effort in the same
    /// run, and the body's mentions are projected by the pump.
    #[tokio::test]
    async fn a_create_is_audited_and_opens_its_effort() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                CREATE,
                json!({
                    "title": "filed",
                    "body": "see [[tsk1]]",
                    "state": "in_progress",
                    "native": { "thread": fx.thread.to_string() },
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "in_progress");
        assert_eq!(out.result["author"], "agent");
        let id: TaskId = out.result["id"].as_str().unwrap().parse().unwrap();
        let executed = out.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(caused, vec!["effort.opened", "work_item.created"]);
        assert!(fx
            .svc
            .effort_store
            .find_open_for_work_item(&work_item_ref(id))
            .await
            .unwrap()
            .is_some());
        fx.svc.event_pump.run_once().await.unwrap();
        let out_refs = fx
            .svc
            .page_ref_store
            .list_outbound(
                oxplow_db::page_ref_projections::KIND_WORK_ITEM,
                &oxplow_db::page_ref_projections::work_item_id(id),
                None,
            )
            .await
            .unwrap();
        assert!(!out_refs.is_empty(), "the body mention was projected");
    }

    /// A queued thread files tasks (tsk466), but not straight into
    /// `in_progress`: that would open an effort.
    #[tokio::test]
    async fn a_queued_thread_files_but_cannot_file_a_claim() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                CREATE,
                json!({ "title": "noted", "native": { "thread": "thr9" } }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "ready");
        let err = fx
            .svc
            .commands
            .run(
                &queued,
                CREATE,
                json!({ "title": "mine now", "native": { "thread": "thr9" }, "state": "in_progress" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let n: i64 = fx
            .svc
            .db
            .transaction(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM task WHERE title = 'mine now'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// Thread 9, queued, in stream 1, as its agent.
    async fn queued_agent(fx: &crate::test_fixtures::EffortFixture) -> Actor {
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (9, 1, 'q', 'queued',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        Actor::Agent {
            thread_id: Some(ThreadId::new(9)),
            stream_id: Some(StreamId::new(1)),
        }
    }

    /// An unknown task fails the run and writes nothing but the error audit.
    #[tokio::test]
    async fn a_failed_transition_logs_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let before = fx
            .svc
            .event_log_store
            .read_after(0, 50)
            .await
            .unwrap()
            .len();
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": "work_item:oxplow:tsk999", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        let after = fx
            .svc
            .event_log_store
            .read_after(0, 50)
            .await
            .unwrap()
            .len();
        assert_eq!(before, after);
    }

    /// How many tasks whose `work_item.native.sort_index` disagrees with
    /// the task row — the restated rows must never fall behind `v_task`.
    async fn stale_native_rows(fx: &crate::test_fixtures::EffortFixture) -> i64 {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_work_item w JOIN v_task t
                   ON w.ref = 'work_item:oxplow:tsk' || t.id
                 WHERE json_extract(w.native, '$.sort_index') IS NOT t.sort_index",
                vec![],
                None,
            )
            .await
            .unwrap();
        match &rows.rows[0][0] {
            oxplow_db::SqlCell::Int(n) => *n,
            other => panic!("{other:?}"),
        }
    }

    /// A list's task refs in order: a thread's, or the backlog's.
    async fn list_order(
        fx: &crate::test_fixtures::EffortFixture,
        thread: Option<ThreadId>,
    ) -> Vec<String> {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT id FROM v_task WHERE thread_id = ?1 OR (?1 IS NULL AND thread_id IS NULL) ORDER BY sort_index, created_at",
                vec![match thread {
                    Some(t) => oxplow_db::SqlCell::Int(t.value()),
                    None => oxplow_db::SqlCell::Null(()),
                }],
                None,
            )
            .await
            .unwrap();
        rows.rows
            .iter()
            .map(|r| match &r[0] {
                oxplow_db::SqlCell::Int(n) => work_item_ref(TaskId::new(*n)),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    async fn file_on(
        fx: &crate::test_fixtures::EffortFixture,
        title: &str,
        thread: Option<ThreadId>,
    ) -> String {
        let mut input = json!({ "title": title });
        if let Some(t) = thread {
            input["native"] = json!({ "thread": t.to_string() });
        }
        fx.svc
            .commands
            .run(&Actor::Human, CREATE, input, false)
            .await
            .unwrap()
            .result["ref"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// P6.E1a: `work_item.reorder` places an item before or after another
    /// in its own list; undo puts it back where it was.
    #[tokio::test]
    async fn reorder_places_an_item_and_undo_puts_it_back() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let t = work_item_ref(fx.task);
        let a = file_on(&fx, "a", Some(fx.thread)).await;
        let b = file_on(&fx, "b", Some(fx.thread)).await;
        let c = file_on(&fx, "c", Some(fx.thread)).await;
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![t.clone(), a.clone(), b.clone(), c.clone()]
        );

        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REORDER,
                json!({ "ref": c, "before": a }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![t.clone(), c.clone(), a.clone(), b.clone()]
        );
        // Every renumbered neighbour's work_item row is restated too.
        assert_eq!(stale_native_rows(&fx).await, 0);
        fx.svc
            .commands
            .run(
                &Actor::Human,
                REORDER,
                json!({ "ref": t, "after": b }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![c.clone(), a.clone(), b.clone(), t.clone()]
        );
        assert_eq!(stale_native_rows(&fx).await, 0);

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![a.clone(), b.clone(), c.clone(), t.clone()]
        );
        assert_eq!(stale_native_rows(&fx).await, 0);

        // The anchor must be in the same list, and there's at most one.
        let other = file_on(&fx, "elsewhere", None).await;
        for input in [
            json!({ "ref": a, "before": other }),
            json!({ "ref": a, "before": b, "after": c }),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, REORDER, input.clone(), false)
                .await
                .unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
    }

    /// `work_item.move` takes an item to another list (its end, or next to
    /// an item there), moving its effort's claim with it; undo brings it
    /// back to its place.
    #[tokio::test]
    async fn move_takes_an_item_to_another_list_and_undo_brings_it_back() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let t = work_item_ref(fx.task);
        let a = file_on(&fx, "a", Some(fx.thread)).await;
        let x = file_on(&fx, "x", None).await;
        let y = file_on(&fx, "y", None).await;

        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                MOVE,
                json!({ "ref": t, "to": "backlog", "before": y }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, None).await,
            vec![x.clone(), t.clone(), y.clone()]
        );
        assert_eq!(list_order(&fx, Some(fx.thread)).await, vec![a.clone()]);
        // Its in-progress claim closed with the move.
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let executed = out.event_id.clone().unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.cause.as_ref() == Some(&executed)
                && e.envelope.event_type == "effort.closed"));

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![t.clone(), a.clone()]
        );
        assert_eq!(list_order(&fx, None).await, vec![x.clone(), y.clone()]);

        fx.svc
            .commands
            .run(
                &Actor::Human,
                MOVE,
                json!({ "ref": x, "to": { "thread": fx.thread.to_string() } }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![t.clone(), a.clone(), x.clone()]
        );
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                MOVE,
                json!({ "ref": y, "to": { "thread": "thr999" } }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thr999"), "{err}");
    }

    /// `work_item.delete` asks first, then removes the task, closing its
    /// open effort, logged as caused by the run.
    #[tokio::test]
    async fn delete_asks_first_then_removes_the_task() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let t = work_item_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "ref": t }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "ref": t }), true)
            .await
            .unwrap();
        assert!(list_order(&fx, Some(fx.thread)).await.is_empty());
        let executed = out.event_id.unwrap();
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(caused, vec!["effort.closed", "work_item.deleted"]);
        let again = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "ref": t }), true)
            .await
            .unwrap_err();
        assert!(matches!(again, CommandError::Invalid { .. }), "{again:?}");
    }
}
