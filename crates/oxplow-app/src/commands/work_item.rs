//! The `work_item.*` commands: the one write surface for every work list's
//! items (`.context/work-items.md`). Each verb runs outside the bus's
//! transaction on the item's provider — the ref's segment, or for
//! `create` the active one (every new item goes to the list the person
//! chose) — through its [`WorkItemVerbs`], the same way for every list,
//! oxplow's own tasks included, with **one** audit row `work_item.<verb>`.
//! A provider the registry doesn't know is refused at `/ref` naming the
//! registered ones; a parent or link target of another provider is refused
//! at its field; a feature the provider doesn't declare (`hierarchy`,
//! `links`, `comments`, `delete`, `ordering`, `lists`) is refused before
//! anything runs.
//!
//! The inputs are the interface's (`oxplow_domain::work_items`): the
//! `v_work_item` columns, a canonical `state` and the provider's
//! `native_state`, and `native` for its own fields. The thread a new item
//! is filed on is core's to resolve ([`filing_thread`]); a loose id is the
//! active list's ([`with_loose_refs`]).
//!
//! What every verb did is logged in the interface's words, the same way
//! for every list ([`canonical_events`]): `work_item.created`, `.edited`,
//! `.state_changed`, `.linked`, `.commented` and `.deleted`, caused by the
//! run's `command.executed`, after the list's own `work_item.recorded` —
//! how what it wrote reaches the interface. An item's state never opens or
//! closes an effort here; the effort policy reacts to
//! `work_item.state_changed` (`crate::effort_policy`).

use crate::commands::ops::Op;
use crate::link_check::LinkDeps;
use oxplow_domain::events::schema::{
    EventType as _, WorkItemCommented, WorkItemCommentedV2, WorkItemCreated, WorkItemCreatedV2,
    WorkItemDeleted, WorkItemDeletedV2, WorkItemEdited, WorkItemEditedV2, WorkItemLinked,
    WorkItemLinkedV2, WorkItemRecorded, WorkItemStateChanged, WorkItemStateChangedV1,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::work_items::{
    provider_of, CanonicalState, WorkItemCommentInput, WorkItemCreateInput, WorkItemDeleteInput,
    WorkItemLinkInput, WorkItemMoveInput, WorkItemReorderInput, WorkItemTransitionInput,
    WorkItemUpdateInput, WorkItemsProvider, WorkItemsRegistry, VERBS,
};
use oxplow_domain::{CommandCall, CommandError, ThreadId};
use serde_json::{json, Value};
use std::sync::Arc;

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, Invocation};

fn parse_thread(raw: &str, field: &str) -> Result<ThreadId, CommandError> {
    raw.parse::<ThreadId>().map_err(|e| CommandError::Invalid {
        field: Some(field.into()),
        message: format!("{e}"),
    })
}

/// The thread a new item is filed on, whatever tracker holds it
/// (tsk1058): the one `create` names, else an agent's own; none for a
/// person who names none (oxplow's backlog).
pub fn filing_thread(
    actor: &oxplow_domain::Actor,
    named: Option<&str>,
) -> Result<Option<ThreadId>, CommandError> {
    match named {
        Some(raw) => parse_thread(raw, "/thread").map(Some),
        None => Ok(actor.thread_id()),
    }
}

/// The fields that name a work item, in any command's input.
const REF_FIELDS: [&str; 6] = [
    "ref",
    "parent_ref",
    "target",
    "work_item",
    "before",
    "after",
];

/// `input` with each work-item field that holds a loose id (`tsk12`)
/// made canonical against the active work list, which declares what its
/// ids look like (`WorkItemsProvider::id_pattern`). A canonical ref is
/// left as it is; a loose id the active list doesn't declare is `Invalid`
/// at its field.
pub(crate) fn with_loose_refs(
    registry: &WorkItemsRegistry,
    mut input: Value,
) -> Result<Value, CommandError> {
    for key in REF_FIELDS {
        let Some(raw) = input.get(key).and_then(Value::as_str) else {
            continue;
        };
        // A ref stays as it is; `parent_ref: ""` means no parent.
        if raw.is_empty() || raw.starts_with("work_item:") {
            continue;
        }
        let canonical = registry
            .loose_ref(raw)
            .map_err(|m| invalid_at(&format!("/{key}"), m))?;
        input[key] = Value::String(canonical);
    }
    Ok(input)
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

/// The active work list: every verb goes to it, whichever implementation
/// it is. One the person chose that isn't running resolved to none.
fn active_provider(registry: &WorkItemsRegistry) -> Result<WorkItemsProvider, CommandError> {
    provider_named(registry, &registry.active(), "").map_err(|e| match e {
        CommandError::Invalid { message, .. } => CommandError::Invalid {
            field: None,
            message: format!("the active work-items provider isn't running: {message}"),
        },
        e => e,
    })
}

/// The active work list, for an input naming `item_ref` (at `field`): it
/// must be that list's item — another list's isn't visible, so it's
/// refused — unless the active one is a sink, which takes any.
fn provider_of_ref(
    registry: &WorkItemsRegistry,
    item_ref: &str,
    field: &str,
) -> Result<WorkItemsProvider, CommandError> {
    let provider = active_provider(registry)?;
    belongs(&provider, item_ref, field)?;
    Ok(provider)
}

/// `item_ref` (an item, a parent, a link target) is `provider`'s own, or
/// `provider` is a sink.
fn belongs(provider: &WorkItemsProvider, item_ref: &str, field: &str) -> Result<(), CommandError> {
    if provider.sink {
        return Ok(());
    }
    let owner = provider_of(item_ref).map_err(|e| invalid_at(field, e.to_string()))?;
    if owner != provider.id {
        return Err(invalid_at(
            field,
            format!(
                "`{item_ref}` is {owner}'s, which isn't the active work list (`{}`)",
                provider.id
            ),
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
    // Always the work list the person and project chose — never another.
    // One that isn't available resolved to none, which files nowhere.
    let provider = active_provider(registry)?;
    if let Some(parent) = &input.parent_ref {
        supports(
            &provider,
            provider.features.hierarchy,
            "hierarchy",
            "/parent_ref",
        )?;
        belongs(&provider, parent, "/parent_ref")?;
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
            belongs(&provider, parent, "/parent_ref")?;
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
    belongs(&provider, &input.target, "/target")?;
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

/// `before` / `after` (a place among the list's items) must be the
/// provider's own items too.
fn place_belongs(
    provider: &WorkItemsProvider,
    before: &Option<String>,
    after: &Option<String>,
) -> Result<(), CommandError> {
    for (field, item) in [("/before", before), ("/after", after)] {
        if let Some(item) = item {
            belongs(provider, item, field)?;
        }
    }
    Ok(())
}

fn reorder_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let input: WorkItemReorderInput = parse(input.clone())?;
    let provider = provider_of_ref(registry, &input.item_ref, "/ref")?;
    supports(&provider, provider.features.ordering, "ordering", "/ref")?;
    place_belongs(&provider, &input.before, &input.after)?;
    Ok(provider)
}

fn move_target(
    registry: &WorkItemsRegistry,
    input: &Value,
) -> Result<WorkItemsProvider, CommandError> {
    let input: WorkItemMoveInput = parse(input.clone())?;
    let provider = provider_of_ref(registry, &input.item_ref, "/ref")?;
    supports(&provider, provider.features.lists, "lists", "/ref")?;
    place_belongs(&provider, &input.before, &input.after)?;
    Ok(provider)
}

/// The provider a `work_item.<verb>` call goes to, as its command routes
/// it — what an effect's automatic retry asks of each step (P10). `None`
/// for a name that isn't a work-items verb, or an input no provider takes.
pub(crate) fn provider_for(
    registry: &WorkItemsRegistry,
    name: &str,
    input: &Value,
) -> Option<WorkItemsProvider> {
    let target: Target = match name.strip_prefix("oxplow.work_item.")? {
        "create" => create_target,
        "update" => update_target,
        "transition" => ref_target,
        "link" => link_target,
        "comment" => comment_target,
        "delete" => delete_target,
        "reorder" => reorder_target,
        "move" => move_target,
        _ => return None,
    };
    target(registry, input).ok()
}

/// A `work_item.<verb>` op: run on the provider `target` names, through
/// its verbs. A `create`'s `thread` is resolved here ([`filing_thread`]);
/// the provider's inverse (a verb) is renamed back to
/// `oxplow.work_item.<verb>`, so an undo dispatches again. With `links`,
/// the result carries the body's `link_warnings` (a create's, and an
/// update's that sets one).
fn dispatching(
    shape: Shape,
    registry: WorkItemsRegistry,
    verb: &'static str,
    target: Target,
    links: Option<LinkDeps>,
) -> Op {
    let external = Arc::new(move |invocation: Invocation, input: Value| {
        let (registry, links) = (registry.clone(), links.clone());
        Box::pin(async move {
            let mut input = with_loose_refs(&registry, input)?;
            let provider = target(&registry, &input)?;
            let mut thread = None;
            if verb == "create" {
                if let Value::Object(fields) = &mut input {
                    let named = fields.get("thread").and_then(Value::as_str);
                    thread = filing_thread(&invocation.actor, named)?;
                    match thread {
                        Some(t) => fields.insert("thread".into(), Value::String(t.to_string())),
                        None => fields.remove("thread"),
                    };
                }
            }
            let out = provider
                .verbs
                .invoke(
                    &invocation.actor,
                    verb,
                    input.clone(),
                    invocation.idempotency_key,
                )
                .await?;
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
            let mut result = out.result;
            // A body's links are checked when there is one to check.
            let body = match verb {
                "create" | "update" => input["body"].as_str().filter(|b| !b.is_empty()),
                _ => None,
            };
            if let (Some(links), Some(body), Value::Object(_)) = (&links, body, &result) {
                let item = written_ref(verb, &input, &result);
                result["link_warnings"] =
                    json!(links.item_warnings(body.to_string(), thread, item).await);
            }
            let mut events = out.events;
            let state = external_state(verb, &input, &result, &events);
            // The interface's answer, the same for every list: the item and,
            // when the verb put it in one, its state.
            if let (Some(state), Value::Object(fields)) = (state, &mut result) {
                fields.insert("state".into(), json!(state));
            }
            events.extend(canonical_events(
                &invocation.actor.source(),
                verb,
                &input,
                &result,
                state,
            ));
            Ok(HandlerOutput {
                result,
                inverse,
                events,
                after_commit: None,
                unchanged: false,
            })
        }) as super::ExternalFuture
    });
    Op::new(
        "work_items.write",
        verb,
        shape.schema,
        shape.undoable,
        Handler::External(external),
    )
}

/// The item a verb wrote: a create's from its result, else the input's
/// `ref`.
fn written_ref(verb: &str, input: &Value, result: &Value) -> Option<String> {
    let source = if verb == "create" { result } else { input };
    source["ref"].as_str().map(str::to_string)
}

/// The state a state verb put its item in, from the item its answer
/// recorded. A provider's prior state isn't read, so a create, a
/// transition and an update naming a state each count as a change; a
/// create its answer recorded no item for is in the state it asked for.
fn external_state(
    verb: &str,
    input: &Value,
    result: &Value,
    events: &[Envelope],
) -> Option<CanonicalState> {
    let names_state = match verb {
        "create" | "transition" => true,
        "update" => input.get("state").is_some() || input.get("native_state").is_some(),
        _ => false,
    };
    if !names_state {
        return None;
    }
    let item_ref = written_ref(verb, input, result)?;
    let recorded = events
        .iter()
        .filter(|e| e.event_type == WorkItemRecorded::TYPE)
        .filter_map(|e| {
            serde_json::from_value::<oxplow_domain::events::schema::WorkItemRecordedV2>(
                e.payload.clone(),
            )
            .ok()
        })
        .find(|r| r.item.item_ref == item_ref)
        .map(|r| r.item.state);
    match verb {
        "create" => recorded.or_else(|| {
            Some(serde_json::from_value(input["state"].clone()).unwrap_or(CanonicalState::Todo))
        }),
        _ => recorded,
    }
}

/// The interface's events for what `verb` did, logged by core the same
/// way for every list (`.context/work-items.md`): from the verb, its
/// input, the list's answer and `state` — the state the verb put the item
/// in, when it changed (always, for a create).
///
/// - `create`: `work_item.created` and `work_item.state_changed`.
/// - `update`: `work_item.edited` naming the fields the input set
///   (`title`, `body`, `parent`, `native.<name>`), and
///   `work_item.state_changed` when the state moved.
/// - `transition`: `work_item.state_changed` when the state moved.
/// - `link`, `comment`, `delete`: `work_item.linked`, `.commented`
///   (naming the answer's `comment`, when it gives one), `.deleted`.
/// - `reorder`, `move`: `work_item.edited` naming `rank`, or `list` and
///   `rank`.
fn canonical_events(
    source: &str,
    verb: &str,
    input: &Value,
    result: &Value,
    state: Option<CanonicalState>,
) -> Vec<Envelope> {
    let Some(item_ref) = written_ref(verb, input, result) else {
        return Vec::new();
    };
    let about = |env: Envelope| env.with_subject([item_ref.clone()]);
    let edited = |fields: Vec<String>| {
        about(Envelope::typed::<WorkItemEdited>(
            source,
            &WorkItemEditedV2 {
                work_item: item_ref.clone(),
                fields,
            },
        ))
    };
    let mut events = Vec::new();
    match verb {
        "create" => {
            if let Some(state) = state {
                events.push(about(Envelope::typed::<WorkItemCreated>(
                    source,
                    &WorkItemCreatedV2 {
                        work_item: item_ref.clone(),
                        state,
                    },
                )));
            }
        }
        "update" => {
            let mut fields: Vec<String> = [
                ("title", "title"),
                ("body", "body"),
                ("parent_ref", "parent"),
            ]
            .into_iter()
            .filter(|(key, _)| input.get(key).is_some())
            .map(|(_, field)| field.to_string())
            .collect();
            if let Some(Value::Object(native)) = input.get("native") {
                fields.extend(native.keys().map(|k| format!("native.{k}")));
            }
            if !fields.is_empty() {
                events.push(edited(fields));
            }
        }
        "link" => {
            let target = input["target"].as_str().unwrap_or_default().to_string();
            events.push(
                Envelope::typed::<WorkItemLinked>(
                    source,
                    &WorkItemLinkedV2 {
                        work_item: item_ref.clone(),
                        target: target.clone(),
                        link_type: input["link_type"].as_str().unwrap_or_default().into(),
                    },
                )
                .with_subject([item_ref.clone(), target]),
            );
        }
        "comment" => events.push(about(Envelope::typed::<WorkItemCommented>(
            source,
            &WorkItemCommentedV2 {
                work_item: item_ref.clone(),
                comment: result["comment"].as_str().map(str::to_string),
            },
        ))),
        "delete" => events.push(about(Envelope::typed::<WorkItemDeleted>(
            source,
            &WorkItemDeletedV2 {
                work_item: item_ref.clone(),
            },
        ))),
        "reorder" => events.push(edited(vec!["rank".into()])),
        "move" => events.push(edited(vec!["list".into(), "rank".into()])),
        _ => {}
    }
    if let Some(to) = state {
        events.push(about(Envelope::typed::<WorkItemStateChanged>(
            source,
            &WorkItemStateChangedV1 {
                work_item: item_ref.clone(),
                to,
            },
        )));
    }
    events
}

fn spec_name(verb: &str) -> String {
    format!("oxplow.work_item.{verb}")
}

/// What a `work_items.write` operation is beyond its handler: the input
/// its handler reads, and whether it returns an inverse.
pub struct Shape {
    pub schema: Value,
    pub undoable: bool,
}

// ---- oxplow.work_item.transition ----

pub const NAME: &str = "oxplow.work_item.transition";

pub fn transition_shape() -> Shape {
    Shape {
        schema: schema::<WorkItemTransitionInput>(),
        undoable: true,
    }
}

pub fn transition_op(registry: WorkItemsRegistry) -> Op {
    dispatching(transition_shape(), registry, "transition", ref_target, None)
}

// ---- oxplow.work_item.create ----

pub const CREATE: &str = "oxplow.work_item.create";

pub fn create_shape() -> Shape {
    Shape {
        schema: schema::<WorkItemCreateInput>(),
        // Undoing a filing would be deleting an item — not what undo is for.
        undoable: false,
    }
}

pub fn create_op(registry: WorkItemsRegistry, links: LinkDeps) -> Op {
    dispatching(
        create_shape(),
        registry,
        "create",
        create_target,
        Some(links),
    )
}

// ---- oxplow.work_item.update ----

pub const UPDATE: &str = "oxplow.work_item.update";

pub fn update_shape() -> Shape {
    Shape {
        schema: schema::<WorkItemUpdateInput>(),
        undoable: true,
    }
}

pub fn update_op(registry: WorkItemsRegistry, links: LinkDeps) -> Op {
    dispatching(
        update_shape(),
        registry,
        "update",
        update_target,
        Some(links),
    )
}

// ---- oxplow.work_item.link ----

pub const LINK: &str = "oxplow.work_item.link";

pub fn link_op(registry: WorkItemsRegistry) -> Op {
    let shape = Shape {
        schema: schema::<WorkItemLinkInput>(),
        undoable: false,
    };
    dispatching(shape, registry, "link", link_target, None)
}

// ---- oxplow.work_item.comment ----

pub const COMMENT: &str = "oxplow.work_item.comment";

pub fn comment_op(registry: WorkItemsRegistry) -> Op {
    let shape = Shape {
        schema: schema::<WorkItemCommentInput>(),
        undoable: false,
    };
    dispatching(shape, registry, "comment", comment_target, None)
}

// ---- oxplow.work_item.delete ----

pub const DELETE: &str = "oxplow.work_item.delete";

pub fn delete_op(registry: WorkItemsRegistry) -> Op {
    let shape = Shape {
        schema: schema::<WorkItemDeleteInput>(),
        undoable: false,
    };
    dispatching(shape, registry, "delete", delete_target, None)
        .confirm_at_least(oxplow_domain::Confirm::Destructive)
}

// ---- oxplow.work_item.reorder / oxplow.work_item.move: a list's order ----

pub const REORDER: &str = "oxplow.work_item.reorder";
pub const MOVE: &str = "oxplow.work_item.move";

pub fn reorder_op(registry: WorkItemsRegistry) -> Op {
    let shape = Shape {
        schema: schema::<WorkItemReorderInput>(),
        undoable: true,
    };
    dispatching(shape, registry, "reorder", reorder_target, None)
}

pub fn move_op(registry: WorkItemsRegistry) -> Op {
    let shape = Shape {
        schema: schema::<WorkItemMoveInput>(),
        undoable: true,
    };
    dispatching(shape, registry, "move", move_target, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The task a run's result names, as its store holds it: the item's
    /// own fields, which the interface's answer (`{ ref, state? }`) leaves
    /// out.
    async fn task_row(svc: &crate::Services, result: &Value) -> Value {
        use oxplow_tasks::TaskStore as _;
        let id = oxplow_tasks::task_of_work_item_ref(result["ref"].as_str().unwrap()).unwrap();
        serde_json::to_value(svc.task_store.get(id).await.unwrap().unwrap()).unwrap()
    }

    /// A column of the oxplow row `sql` (one `?1`, the id) reads.
    async fn column(svc: &crate::Services, sql: &'static str, id: String) -> Value {
        svc.db
            .read(move |tx| {
                tx.query_row(sql, [id], |r| r.get::<_, String>(0))
                    .map(Value::String)
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }
    use oxplow_db::EffortStore as _;
    use oxplow_domain::Actor;
    use oxplow_domain::StreamId;
    use oxplow_tasks::TaskId;
    use oxplow_tasks::{work_item_ref, TaskPriority, TaskStatus};

    /// tsk775: filing or editing an oxplow task answers with the
    /// `[[…]]` links in its body that don't resolve, as a note does, so an
    /// agent fixes a broken link in the same turn.
    #[tokio::test]
    async fn a_task_body_answers_with_its_broken_links() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let run = |name: &'static str, input: Value| {
            let bus = fx.svc.commands.clone();
            async move { bus.run(&Actor::Human, name, input, false).await.unwrap() }
        };
        let filed = run(
            CREATE,
            json!({ "title": "linky", "body": "see [[tsk99999]] and [[#12]]" }),
        )
        .await;
        let targets = |v: &Value| -> Vec<String> {
            v["link_warnings"]
                .as_array()
                .unwrap_or_else(|| panic!("no link_warnings in {v}"))
                .iter()
                .map(|w| w["target"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(targets(&filed.result), vec!["tsk99999", "#12"]);
        let item = filed.result["ref"].as_str().unwrap().to_string();
        let edited = run(
            UPDATE,
            json!({ "ref": item, "body": "now see [[tsk99998]]" }),
        )
        .await;
        assert_eq!(targets(&edited.result), vec!["tsk99998"]);
        let fixed = run(UPDATE, json!({ "ref": item, "body": "no links" })).await;
        assert_eq!(targets(&fixed.result), Vec::<String>::new());
    }

    /// P5.C2: the commands take canonical refs and refuse another
    /// provider's, naming the registered ones.
    #[tokio::test]
    async fn another_lists_ref_is_refused_naming_the_active_list() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for (name, input) in [
            (
                NAME,
                json!({ "ref": "work_item:issues:ENG-12", "to": "done" }),
            ),
            (
                UPDATE,
                json!({ "ref": "work_item:issues:ENG-12", "title": "x" }),
            ),
            (
                COMMENT,
                json!({ "ref": "work_item:issues:ENG-12", "body": "x" }),
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
                        message,
                        "`work_item:issues:ENG-12` is issues's, which isn't the active work list (`oxplow`)",
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
                // Not a ref, nor an id the active list declares.
                json!({ "ref": "ENG-12", "to": "done" }),
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
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "archived");
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
        use oxplow_tasks::TaskStore as _;
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
                    "thread": fx.thread.to_string(),
                    "native": { "priority": "high" },
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(task_row(&fx.svc, &out.result).await["priority"], "high");
        assert_eq!(
            task_row(&fx.svc, &out.result).await["thread_id"],
            fx.thread.to_string()
        );
        assert_eq!(
            task_row(&fx.svc, &out.result).await["description"],
            "the body"
        );
        for (input, field) in [
            (
                json!({ "title": "x", "native": { "bogus": 1 } }),
                Some("/native"),
            ),
            (json!({ "title": "x", "priority": "high" }), None),
            (json!({ "title": "x", "thread": "nope" }), Some("/thread")),
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

    /// tsk1000: a comment is recorded as its author — an effect's or
    /// oxplow's own isn't shown as the person's.
    #[tokio::test]
    async fn a_comment_is_recorded_as_whoever_made_it() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let item = oxplow_tasks::work_item_ref(fx.task);
        for (actor, author) in [
            (Actor::Human, "user"),
            (
                Actor::Effect {
                    effect: "acme/notify".into(),
                },
                "effect:acme/notify",
            ),
            (Actor::System, "oxplow"),
        ] {
            let commented = fx
                .svc
                .commands
                .run(
                    &actor,
                    COMMENT,
                    json!({ "ref": item, "body": "noted" }),
                    false,
                )
                .await
                .unwrap();
            let note = commented.result["comment"].as_str().unwrap();
            let id = note.trim_start_matches("not").to_string();
            assert_eq!(
                column(&fx.svc, "SELECT author FROM task_note WHERE id = ?1", id).await,
                author,
                "{actor:?}"
            );
        }
    }

    /// The active work list is resolved from the config as it is now — a
    /// `create` right after a person chose another files there, with no
    /// reconcile in between; one that isn't available resolves to none, and
    /// filing says so.
    /// A loose id the active work list declares (`tsk12` for oxplow's
    /// tasks) is that list's item, in every work-item ref field and in an
    /// effort's link; one it doesn't declare is refused naming the shapes.
    #[tokio::test]
    async fn a_loose_id_resolves_against_the_active_work_list() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let loose = fx.task.to_string();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": loose, "to": "blocked" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "blocked");
        let child = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "child", "parent_ref": loose }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &child.result).await["parent_id"],
            json!(loose)
        );
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::effort::LINK,
                json!({ "effort": oxplow_domain::refs::build::effort_ref(fx.effort), "work_item": loose }),
                false,
            )
            .await
            .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "ref": "ENG-1", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        match err {
            CommandError::Invalid { field, message } => {
                assert_eq!(field.as_deref(), Some("/ref"));
                assert!(message.contains("tsk"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_create_reads_the_active_provider_the_config_names_now() {
        let fx = crate::test_fixtures::services_with_effort().await;
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "issues".into());
        // The chosen one isn't running: none is, and files nowhere.
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "where?" }), false)
            .await
            .unwrap();
        assert_eq!(out.result["tracked"], json!(false));
        assert_eq!(fx.svc.work_items.active(), oxplow_domain::capability::NONE);
    }

    /// P7.A2, tsk1058: every `create` files on the active tracker — oxplow's
    /// by default; one that isn't running is an error naming it, never a
    /// fallback — and no caller can name another.
    #[tokio::test]
    async fn every_create_files_on_the_active_tracker() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let filed = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "here" }), false)
            .await
            .unwrap();
        assert!(filed.result["ref"]
            .as_str()
            .unwrap()
            .starts_with("work_item:oxplow:"));
        // Naming a provider isn't an input: the person chose the tracker.
        let named = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "provider": "oxplow", "title": "named" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(named, CommandError::Invalid { .. }), "{named:?}");
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "tracker".into());
        let before = list_order(&fx, None).await.len();
        // Never another list: the chosen one isn't running, so none files
        // it nowhere.
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "where?" }), false)
            .await
            .unwrap();
        assert_eq!(out.result["tracked"], json!(false));
        assert_eq!(list_order(&fx, None).await.len(), before, "nothing filed");
    }

    /// tsk1058: the thread a work item is filed on is a common field, not
    /// a tracker's own: an agent's create lands on its own thread unless it
    /// names one, a person's on the one named (else the backlog).
    #[tokio::test]
    async fn a_create_is_filed_on_a_thread_by_the_common_field() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let mine = fx
            .svc
            .commands
            .run(&agent, CREATE, json!({ "title": "mine" }), false)
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &mine.result).await["thread_id"],
            fx.thread.to_string()
        );
        let named = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "theirs", "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &named.result).await["thread_id"],
            fx.thread.to_string()
        );
        let backlog = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "later" }), false)
            .await
            .unwrap();
        assert!(
            task_row(&fx.svc, &backlog.result).await["thread_id"].is_null(),
            "{}",
            backlog.result
        );
        // oxplow's own fields are its priority only.
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "old", "native": { "thread": fx.thread.to_string() } }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    /// A link made by a person (no thread of their own) belongs to the
    /// linked task's thread; two backlog tasks can't be linked that way.
    #[tokio::test]
    async fn a_persons_link_takes_the_tasks_thread() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(
            task_row(&fx.svc, &out.result).await["thread_id"],
            fx.thread.to_string()
        );
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
        use oxplow_tasks::TaskStore as _;
        let fx = crate::test_fixtures::services_with_task_effort().await;
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

    /// `oxplow.work_item.link` and `oxplow.work_item.comment` write the link and the
    /// note, each with its event, caused by the run.
    #[tokio::test]
    async fn links_and_comments_are_commands_with_their_events() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        let from_id = from.rsplit("tsk").next().unwrap().to_string();
        assert_eq!(
            column(
                &fx.svc,
                "SELECT link_type || ' ' || thread_id FROM task_link WHERE from_item_id = ?1",
                from_id
            )
            .await,
            format!("blocks {}", fx.thread.value())
        );
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
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        // The comment event names the note by the list's own id for it.
        let note = commented.result["comment"].as_str().unwrap().to_string();
        assert!(note.starts_with("not"), "{note}");
        assert_eq!(
            column(
                &fx.svc,
                "SELECT author FROM task_note WHERE id = ?1",
                note.trim_start_matches("not").to_string()
            )
            .await,
            "agent"
        );
        let logged = events
            .iter()
            .find(|e| {
                e.envelope.cause == commented.event_id
                    && e.envelope.event_type == "work_item.commented"
            })
            .unwrap();
        assert_eq!(
            logged.envelope.payload,
            json!({ "work_item": from, "comment": note })
        );
        for (out, event_type) in [
            (&linked, "work_item.linked"),
            (&commented, "work_item.commented"),
        ] {
            let caused: Vec<&str> = events
                .iter()
                .filter(|e| e.envelope.cause == out.event_id)
                .map(|e| e.envelope.event_type.as_str())
                .collect();
            assert_eq!(caused, vec!["work_item.recorded", event_type]);
        }
        // Both are the item's page refs once the pump restates it from
        // the interface.
        fx.svc.event_pump.run_once().await.unwrap();
        let id = from.strip_prefix("work_item:").unwrap().to_string();
        let edges: Vec<(String, String)> = fx
            .svc
            .db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT ref_type, target_id FROM page_ref
                         WHERE source_kind = 'work_item' AND source_id = ?1 ORDER BY ref_type",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(oxplow_db::map_sql_err);
                rows
            })
            .await
            .unwrap();
        let other_id = other.strip_prefix("work_item:").unwrap();
        assert!(
            edges.contains(&("comment_file_ref".into(), "src/lib.rs".into())),
            "{edges:?}"
        );
        assert!(
            edges.contains(&("work_item_link:blocks".into(), other_id.into())),
            "{edges:?}"
        );
        let err = fx
            .svc
            .commands
            .run(&agent, COMMENT, json!({ "ref": from, "body": "  " }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("body"), "{err}");
    }

    /// A transition is one transaction with its audit: the status and
    /// core's `work_item.state_changed` carry the actor's source and are
    /// caused by the run's `command.executed`.
    /// The effort policy closes the item's effort after it, as a reaction.
    #[tokio::test]
    async fn a_transition_commits_with_its_audit_and_names_its_cause() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
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
        assert_eq!(task_row(&fx.svc, &outcome.result).await["status"], "done");
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
                // The list's own record, then core's event.
                ("work_item.recorded", "provider:oxplow"),
                ("work_item.state_changed", source.as_str()),
            ]
        );
        assert!(events
            .iter()
            .any(|e| e.envelope.id == executed && e.envelope.event_type == "command.executed"));
        fx.svc
            .event_pump
            .settle(
                &[crate::effort_policy::NAME],
                std::time::Duration::from_secs(5),
            )
            .await;
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert!(effort.ended_at.is_some());
    }

    /// `oxplow.work_item.update`: fields and state commit together, audited and
    /// undoable — an undo restores both.
    #[tokio::test]
    async fn an_update_edits_fields_and_state_atomically_and_undoes() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(task_row(&fx.svc, &out.result).await["title"], "renamed");
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "blocked");
        assert_eq!(task_row(&fx.svc, &out.result).await["priority"], "urgent");
        let executed = out.event_id.clone().unwrap();
        let events = fx.svc.event_log_store.read_after(0, 100).await.unwrap();
        let caused: Vec<(&str, &serde_json::Value)> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .filter(|e| e.envelope.event_type != "work_item.recorded")
            .map(|e| (e.envelope.event_type.as_str(), &e.envelope.payload))
            .collect();
        let item = work_item_ref(fx.task);
        assert_eq!(
            caused,
            vec![
                (
                    "work_item.edited",
                    &json!({ "work_item": item, "fields": ["title", "native.priority"] })
                ),
                (
                    "work_item.state_changed",
                    &json!({ "work_item": item, "to": "blocked" })
                ),
            ]
        );

        bus.undo(&agent, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_tasks::TaskStore as _;
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
        // A task changes lists with `oxplow.work_item.move`; the thread isn't a
        // native field.
        assert!(
            matches!(&err, CommandError::Invalid { field, .. } if field.as_deref() == Some("/native")),
            "{err:?}"
        );
    }

    /// A queued thread (tsk466) may edit a task and move it anywhere but
    /// `in_progress`: task bookkeeping isn't a claim on the worktree.
    #[tokio::test]
    async fn a_queued_thread_edits_and_finishes_tasks() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "done");
        use oxplow_tasks::TaskStore as _;
        let row = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("renamed", TaskStatus::Done)
        );
    }

    /// Any thread may start a task: a task's state is a record, not a
    /// claim on the worktree (only edits are guarded, by isolation).
    #[tokio::test]
    async fn a_queued_thread_starts_and_files_tasks_in_progress() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let queued = queued_agent(&fx).await;
        let later = file_on(&fx, "later", Some(fx.thread)).await;
        fx.svc
            .commands
            .run(
                &queued,
                NAME,
                json!({ "ref": later, "to": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                CREATE,
                json!({ "title": "mine now", "thread": "thr9", "state": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &out.result).await["status"],
            "in_progress"
        );
    }

    /// `oxplow.work_item.create`: filing a task is audited to the actor; filed
    /// straight into `in_progress`, the effort policy then switches the
    /// thread's effort to it; the body's mentions are projected by the pump.
    #[tokio::test]
    async fn a_create_is_audited_and_its_start_switches_the_effort() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
                    "thread": fx.thread.to_string(),
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &out.result).await["status"],
            "in_progress"
        );
        assert_eq!(task_row(&fx.svc, &out.result).await["author"], "agent");
        let id: TaskId =
            oxplow_tasks::task_of_work_item_ref(out.result["ref"].as_str().unwrap()).unwrap();
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
                "work_item.recorded",
                "work_item.created",
                "work_item.state_changed"
            ]
        );
        // The fixture's effort closes as the thread switches to this one.
        fx.svc
            .event_pump
            .settle(
                &[crate::effort_policy::NAME],
                std::time::Duration::from_secs(5),
            )
            .await;
        let fixture_effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fixture_effort.closed_by.as_deref(), Some("switch"));
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
                &oxplow_tasks::work_item_id(id),
                None,
            )
            .await
            .unwrap();
        assert!(!out_refs.is_empty(), "the body mention was projected");
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

    /// How many tasks whose `work_item.rank` disagrees with the task
    /// row's `sort_index` — the restated rows must never fall behind the
    /// task table.
    async fn stale_ranks(fx: &crate::test_fixtures::EffortFixture) -> i64 {
        fx.svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM v_work_item w JOIN task t
                       ON w.ref = 'work_item:oxplow:tsk' || t.id
                     WHERE w.rank IS NOT t.sort_index",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A list's work item refs in order: a thread's, or the backlog's.
    async fn list_order(
        fx: &crate::test_fixtures::EffortFixture,
        thread: Option<ThreadId>,
    ) -> Vec<String> {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT ref FROM v_work_item WHERE thread_id = ?1 OR (?1 IS NULL AND thread_id IS NULL) ORDER BY rank, created_at",
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
                oxplow_db::SqlCell::Text(s) => s.clone(),
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
            input["thread"] = json!(t.to_string());
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

    /// `parent_ref: ""` detaches an item from its parent, as the input
    /// documents; undo puts the parent back.
    #[tokio::test]
    async fn an_empty_parent_ref_detaches_and_undo_reattaches() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let parent = work_item_ref(fx.task);
        let child = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "child", "parent_ref": parent }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &child.result).await["parent_id"],
            json!(fx.task)
        );
        let detached = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                UPDATE,
                json!({ "ref": child.result["ref"], "parent_ref": "" }),
                false,
            )
            .await
            .unwrap();
        assert!(task_row(&fx.svc, &child.result).await["parent_id"].is_null());
        fx.svc
            .commands
            .undo(&Actor::Human, detached.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &child.result).await["parent_id"],
            json!(fx.task)
        );
    }

    /// P6.E1a: `oxplow.work_item.reorder` places an item before or after another
    /// in its own list; undo puts it back where it was.
    #[tokio::test]
    async fn reorder_places_an_item_and_undo_puts_it_back() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(stale_ranks(&fx).await, 0);
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
        assert_eq!(stale_ranks(&fx).await, 0);

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            list_order(&fx, Some(fx.thread)).await,
            vec![a.clone(), b.clone(), c.clone(), t.clone()]
        );
        assert_eq!(stale_ranks(&fx).await, 0);

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

    /// `oxplow.work_item.move` takes an item to another list (its end, or next to
    /// an item there); undo brings it back to its place.
    #[tokio::test]
    async fn move_takes_an_item_to_another_list_and_undo_brings_it_back() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        // The thread's effort is the thread's: a move leaves it open.
        let effort = fx
            .svc
            .effort_store
            .get_effort(&fx.effort)
            .await
            .unwrap()
            .unwrap();
        assert!(effort.ended_at.is_none());

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

    /// `oxplow.work_item.delete` asks first, then removes the task, logged as
    /// caused by the run.
    #[tokio::test]
    async fn delete_asks_first_then_removes_the_task() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
        assert_eq!(caused, vec!["work_item.recorded", "work_item.deleted"]);
        let again = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "ref": t }), true)
            .await
            .unwrap_err();
        assert!(matches!(again, CommandError::Invalid { .. }), "{again:?}");
    }
}
