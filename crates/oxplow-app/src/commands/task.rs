//! The `task.*` commands: the one write surface for every task tracker's
//! items (`.context/tasks.md`). Each verb runs outside the bus's
//! transaction on the item's provider — the ref's segment, or for
//! `create` the active one (every new item goes to the list the person
//! chose) — through its [`TaskVerbs`], the same way for every list,
//! oxplow's own tasks included, with **one** audit row `task.<verb>`.
//! A provider the registry doesn't know is refused at `/ref` naming the
//! registered ones; a parent or link target of another provider is refused
//! at its field; a feature the provider doesn't declare (`hierarchy`,
//! `links`, `comments`, `delete`) is refused before anything runs.
//!
//! The inputs are the interface's (`oxplow_domain::tasks`): the
//! `v_task` columns: a `stage` (one the list declares, or a bucket
//! for its default — core resolves it, [`resolve_stages`]) and `native`
//! for its own fields; a loose id is the active list's
//! ([`with_loose_refs`]).
//!
//! **Assignment is core's** ([`assign_op`], [`reorder_op`]): who is going
//! to work on an item — a thread, the person or nobody — and its place in
//! that assignee's queue, for every list, logged as `task.assigned`
//! and kept by the `tasks.project` consumer. A create may name its
//! assignee (the list never sees it); a verb that closes an item releases
//! its assignment.
//!
//! What every verb did is logged in the interface's words, the same way
//! for every list ([`canonical_events`]): `task.created`, `.edited`,
//! `.state_changed`, `.linked`, `.commented` and `.deleted`, caused by the
//! run's `command.executed`, after the list's own `task.recorded` —
//! how what it wrote reaches the interface.

use crate::commands::ops::Op;
use crate::link_check::LinkDeps;
use oxplow_domain::events::schema::{
    EventType as _, TaskAssigned, TaskAssignedV1, TaskCommented, TaskCommentedV2, TaskCreated,
    TaskCreatedV2, TaskDeleted, TaskDeletedV2, TaskEdited, TaskEditedV2, TaskLinked, TaskLinkedV2,
    TaskRecorded, TaskStateChanged, TaskStateChangedV1, TaskUnlinked, TaskUnlinkedV1,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::tasks::{
    provider_of, Assignee, Bucket, TaskAssignInput, TaskCommentInput, TaskCreateInput,
    TaskDeleteInput, TaskLinkInput, TaskReorderInput, TaskTracker, TaskTrackers,
    TaskTransitionInput, TaskUpdateInput,
};
use oxplow_domain::{CommandCall, CommandError};
use serde_json::{json, Value};
use std::sync::Arc;

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, Invocation, TxCtx};

/// The fields that name a task, in any command's input.
const URI_FIELDS: [&str; 6] = ["uri", "parent_uri", "target", "task", "before", "after"];

/// `input` with each task field that holds a loose id (`tsk12`)
/// made canonical against the active task tracker, which declares what its
/// ids look like (`TaskTracker::id_pattern`). A canonical ref is
/// left as it is; a loose id the active list doesn't declare is `Invalid`
/// at its field.
pub(crate) fn with_loose_refs(
    registry: &TaskTrackers,
    mut input: Value,
) -> Result<Value, CommandError> {
    for key in URI_FIELDS {
        let Some(raw) = input.get(key).and_then(Value::as_str) else {
            continue;
        };
        // A ref stays as it is (a link's target may be any ref);
        // `parent_uri: ""` means no parent.
        if raw.is_empty() || oxplow_domain::uri::build::resource_of(raw).is_some() {
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
    registry: &TaskTrackers,
    id: &str,
    field: &str,
) -> Result<TaskTracker, CommandError> {
    registry
        .get(id)
        .map_err(|e| invalid_at(field, e.to_string()))
}

/// The active task tracker: every verb goes to it, whichever implementation
/// it is. One the person chose that isn't running resolved to none.
fn active_provider(registry: &TaskTrackers) -> Result<TaskTracker, CommandError> {
    provider_named(registry, &registry.active(), "").map_err(|e| match e {
        CommandError::Invalid { message, .. } => CommandError::Invalid {
            field: None,
            message: format!("the active task tracker provider isn't running: {message}"),
        },
        e => e,
    })
}

/// The active task tracker, for an input naming `item_ref` (at `field`): it
/// must be that list's item — another list's isn't visible, so it's
/// refused — unless the active one is a sink, which takes any.
fn provider_of_ref(
    registry: &TaskTrackers,
    item_ref: &str,
    field: &str,
) -> Result<TaskTracker, CommandError> {
    let provider = active_provider(registry)?;
    belongs(&provider, item_ref, field)?;
    Ok(provider)
}

/// `item_ref` (an item, a parent, a link target) is `provider`'s own, or
/// `provider` is a sink.
fn belongs(provider: &TaskTracker, item_ref: &str, field: &str) -> Result<(), CommandError> {
    if provider.sink {
        return Ok(());
    }
    let owner = provider_of(item_ref).map_err(|e| invalid_at(field, e.to_string()))?;
    if owner != provider.id {
        return Err(invalid_at(
            field,
            format!(
                "`{item_ref}` is {owner}'s, which isn't the active task tracker (`{}`)",
                provider.id
            ),
        ));
    }
    Ok(())
}

/// The provider must declare `feature` (its `v_capability_provider`
/// flag) for this input.
fn supports(
    provider: &TaskTracker,
    has: bool,
    feature: &str,
    field: &str,
) -> Result<(), CommandError> {
    if has {
        Ok(())
    } else {
        Err(invalid_at(
            field,
            format!("{} tasks don't support {feature}", provider.id),
        ))
    }
}

/// Who answers a verb's input: the provider, checked for what the input
/// asks of it.
type Target = fn(&TaskTrackers, &Value) -> Result<TaskTracker, CommandError>;

/// An item always has a title: a blank one is refused at `/title`, whichever
/// list it is for.
fn not_blank(title: &str) -> Result<(), CommandError> {
    if title.trim().is_empty() {
        return Err(invalid_at("/title", "a title can't be empty".into()));
    }
    Ok(())
}

fn create_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let input: TaskCreateInput = parse(input.clone())?;
    not_blank(&input.title)?;
    // Always the task tracker the person and project chose — never another.
    // One that isn't available resolved to none, which files nowhere.
    let provider = active_provider(registry)?;
    if let Some(parent) = &input.parent_uri {
        supports(
            &provider,
            provider.features.hierarchy,
            "hierarchy",
            "/parent_uri",
        )?;
        belongs(&provider, parent, "/parent_uri")?;
    }
    if !input.links.is_empty() {
        supports(&provider, provider.features.links, "links", "/links")?;
        for (n, link) in input.links.iter().enumerate() {
            target_belongs(&provider, &link.target, &format!("/links/{n}/target"))?;
        }
    }
    Ok(provider)
}

/// A link's target: an item must be `provider`'s own; any other ref is
/// the item's to point at (its kind is checked against the vocabulary
/// when the command runs, [`check_link_targets`]).
fn target_belongs(provider: &TaskTracker, target: &str, field: &str) -> Result<(), CommandError> {
    match oxplow_domain::uri::build::resource_of(target) {
        None => Err(invalid_at(
            field,
            format!("`{target}` isn't a URI (oxplow:<extension>/<resource>?…)"),
        )),
        Some(kind) if kind == oxplow_domain::uri::resources::TASK => {
            belongs(provider, target, field)
        }
        Some(_) => Ok(()),
    }
}

/// The link targets an input names, with their fields: a `link` /
/// `unlink`'s `target`, a `create`'s `links[n].target`.
fn link_targets(verb: &str, input: &Value) -> Vec<(String, String)> {
    match verb {
        "link" | "unlink" => input["target"]
            .as_str()
            .map(|t| vec![("/target".to_string(), t.to_string())])
            .unwrap_or_default(),
        "create" => input["links"]
            .as_array()
            .map(|links| {
                links
                    .iter()
                    .enumerate()
                    .filter_map(|(n, l)| {
                        l["target"]
                            .as_str()
                            .map(|t| (format!("/links/{n}/target"), t.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Every link target is a ref of a kind the running vocabulary has (an
/// uninstalled extension's kind is refused when linking — a link already
/// made to it may dangle).
fn check_link_targets(
    vocabulary: &oxplow_domain::vocabulary::VocabularyHandle,
    verb: &str,
    input: &Value,
) -> Result<(), CommandError> {
    let current = vocabulary.current();
    for (field, target) in link_targets(verb, input) {
        oxplow_domain::uri::validate_uri(&current.resources, &target)
            .map_err(|e| invalid_at(&field, e.to_string()))?;
    }
    Ok(())
}

fn update_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let input: TaskUpdateInput = parse(input.clone())?;
    if let Some(title) = &input.title {
        not_blank(title)?;
    }
    let provider = provider_of_ref(registry, &input.uri, "/uri")?;
    if let Some(parent) = &input.parent_uri {
        supports(
            &provider,
            provider.features.hierarchy,
            "hierarchy",
            "/parent_uri",
        )?;
        if !parent.is_empty() {
            belongs(&provider, parent, "/parent_uri")?;
        }
    }
    Ok(provider)
}

fn ref_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let item_ref = input["uri"].as_str().unwrap_or_default();
    provider_of_ref(registry, item_ref, "/uri")
}

fn link_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let input: TaskLinkInput = parse(input.clone())?;
    let provider = provider_of_ref(registry, &input.uri, "/uri")?;
    supports(&provider, provider.features.links, "links", "/uri")?;
    target_belongs(&provider, &input.target, "/target")?;
    Ok(provider)
}

fn comment_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let provider = ref_target(registry, input)?;
    supports(&provider, provider.features.comments, "comments", "/uri")?;
    Ok(provider)
}

fn delete_target(registry: &TaskTrackers, input: &Value) -> Result<TaskTracker, CommandError> {
    let provider = ref_target(registry, input)?;
    supports(&provider, provider.features.delete, "delete", "/uri")?;
    Ok(provider)
}

/// The provider a `task.<verb>` call goes to, as its command routes
/// it — what an effect's automatic retry asks of each step (P10). `None`
/// for a name that isn't a task tracker verb, or an input no provider takes.
pub(crate) fn provider_for(
    registry: &TaskTrackers,
    name: &str,
    input: &Value,
) -> Option<TaskTracker> {
    let target: Target = match name.strip_prefix("oxplow.task.")? {
        "create" => create_target,
        "update" => update_target,
        "transition" => ref_target,
        "link" | "unlink" => link_target,
        "comment" => comment_target,
        "delete" => delete_target,
        _ => return None,
    };
    target(registry, input).ok()
}

/// A `task.<verb>` op: run on the provider `target` names, through
/// its verbs. A `create`'s `assignee` is core's: checked, kept from the
/// list, and logged as `task.assigned` once the item exists. A verb
/// that closes an assigned item releases it (`task.assigned` to
/// nobody). The provider's inverse (a verb) is renamed back to
/// `oxplow.task.<verb>`, so an undo dispatches again. With `links`,
/// the result carries the body's `link_warnings` (a create's, and an
/// update's that sets one).
fn dispatching(
    shape: Shape,
    registry: TaskTrackers,
    verb: &'static str,
    target: Target,
    links: Option<LinkDeps>,
    reads: Option<oxplow_db::Database>,
) -> Op {
    let external = Arc::new(move |invocation: Invocation, input: Value| {
        let (registry, links, reads) = (registry.clone(), links.clone(), reads.clone());
        Box::pin(async move {
            let mut input = with_loose_refs(&registry, input)?;
            let provider = target(&registry, &input)?;
            if let Some(links) = &links {
                check_link_targets(&links.vocabulary, verb, &input)?;
            }
            // Who a new item is for is core's record, never the list's.
            let mut assignee = None;
            if verb == "create" {
                if let Value::Object(fields) = &mut input {
                    if let Some(raw) = fields.remove("assignee") {
                        let raw = raw.as_str().ok_or_else(|| {
                            invalid_at("/assignee", "an assignee is a URI".into())
                        })?;
                        let to = Assignee::parse(raw).map_err(|m| invalid_at("/assignee", m))?;
                        if let Some(db) = &reads {
                            let checked = to.clone();
                            if let Some(problem) =
                                db.read(move |tx| assignee_problem(tx, &checked)).await?
                            {
                                return Err(invalid_at("/assignee", problem));
                            }
                        }
                        assignee = Some(to.uri());
                    }
                }
            }
            // A stage is the list's: a bucket's name means its default, and
            // the list receives its own stage's id.
            resolve_stages(&provider, verb, &mut input)?;
            // The stage and bucket it's in before the verb, and who it's
            // assigned to: a verb that leaves its bucket moved nothing, and
            // one that closes it releases its assignee.
            let prior = match (&reads, input["uri"].as_str()) {
                (Some(db), Some(item)) if verb != "create" => prior_of(db, item).await?,
                _ => Prior::default(),
            };
            let out = provider
                .verbs
                .invoke(
                    oxplow_domain::tasks::VerbCall {
                        run: invocation.run.clone(),
                        idempotency_key: invocation.idempotency_key.clone(),
                    },
                    verb,
                    input.clone(),
                )
                .await?;
            let inverse = out
                .inverse
                .map(|c| {
                    if oxplow_domain::capability::TASK_TRACKER
                        .verb(&c.name)
                        .is_none()
                    {
                        return Err(CommandError::Failed {
                            message: format!(
                                "provider `{}` returned an inverse `{}`, which isn't a task tracker \
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
                let thread = invocation.actor().thread_id();
                result["link_warnings"] =
                    json!(links.item_warnings(body.to_string(), thread, item).await);
            }
            let mut events = out.events;
            let stage = recorded_stage(verb, &input, &result, &events);
            let state = stage
                .as_deref()
                .and_then(|st| oxplow_domain::tasks::bucket_of(&provider.stages, st));
            // The interface's answer, the same for every list: the item and,
            // when the verb put it in one, its stage and bucket.
            if let (Some(stage), Some(state), Value::Object(fields)) = (&stage, state, &mut result)
            {
                fields.insert("stage".into(), json!(stage));
                fields.insert("state".into(), json!(state));
            }
            // `task.state_changed` says the bucket moved: not for a
            // verb that left it there. A move between two of a bucket's
            // stages is an edit of the stage.
            let moved = state.filter(|s| prior.state != Some(*s));
            let restaged = stage.is_some() && stage != prior.stage && moved.is_none();
            let source = invocation.actor().source();
            events.extend(canonical_events(
                &source, verb, &input, &result, moved, restaged,
            ));
            if let Some(item) = written_ref(verb, &input, &result) {
                let closed = state.is_some_and(Bucket::is_closed);
                // Filed for someone: assigned, unless it was filed closed.
                let named = assignee.is_some();
                if let (Some(to), false) = (assignee, closed) {
                    events.push(assigned(&source, &item, None, Some(to), None));
                }
                // Closed: who worked on it is its requests, so it's
                // nobody's to work on now.
                if let (Some(from), true) = (&prior.assignee, closed && verb != "create") {
                    events.push(assigned(&source, &item, Some(from.clone()), None, None));
                }
                // An agent that starts a task takes it: it's assigned to the
                // agent's thread, from whoever had it — one it files straight
                // into progress too, unless it names whose it is. A person's
                // start assigns nothing.
                let started = moved == Some(Bucket::InProgress) && !(verb == "create" && named);
                if let (true, Some(thread)) = (started, agent_thread(&invocation)) {
                    let mine = oxplow_domain::uri::build::thread_uri(thread);
                    if prior.assignee.as_deref() != Some(mine.as_str()) {
                        events.push(assigned(&source, &item, prior.assignee, Some(mine), None));
                    }
                }
            }
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
        "task_tracker.write",
        verb,
        shape.schema,
        shape.undoable,
        Handler::External(external),
    )
}

/// The thread of the agent running `invocation`; none for a person or a
/// page.
fn agent_thread(invocation: &Invocation) -> Option<oxplow_domain::ThreadId> {
    let actor = invocation.actor();
    (actor.kind() == oxplow_domain::events::schema::ActorKind::Agent)
        .then(|| actor.thread_id())
        .flatten()
}

/// What an item was before a verb: its stage, its bucket and who it's
/// assigned to, as the interface shows them; empty for one it doesn't.
#[derive(Debug, Default)]
struct Prior {
    stage: Option<String>,
    state: Option<Bucket>,
    assignee: Option<String>,
}

/// `item_ref` as the interface shows it before the verb (`v_task`).
async fn prior_of(db: &oxplow_db::Database, item_ref: &str) -> Result<Prior, CommandError> {
    use rusqlite::OptionalExtension as _;
    let item = item_ref.to_string();
    let row: Option<(String, String, Option<String>)> = db
        .read(move |tx| {
            tx.query_row(
                "SELECT stage, state, assignee FROM v_task WHERE uri = ?1",
                [&item],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await?;
    Ok(match row {
        Some((stage, state, assignee)) => Prior {
            stage: Some(stage),
            state: Bucket::named(&state),
            assignee,
        },
        None => Prior::default(),
    })
}

/// `input`'s stage — `transition`'s `to`, `create`'s and `update`'s
/// `stage` — as one of `provider`'s stages: a stage id stays, a bucket's
/// name becomes that bucket's default, anything else is refused at its
/// field naming the list's stages. A create naming none starts in
/// `todo`'s default.
fn resolve_stages(
    provider: &TaskTracker,
    verb: &str,
    input: &mut Value,
) -> Result<(), CommandError> {
    let field = match verb {
        "transition" => "to",
        "create" | "update" => "stage",
        _ => return Ok(()),
    };
    let Value::Object(fields) = input else {
        return Ok(());
    };
    let named = match fields.get(field) {
        Some(Value::String(name)) => Some(name.clone()),
        Some(other) => {
            return Err(invalid_at(
                &format!("/{field}"),
                format!("a stage is its id (a string), not {other}"),
            ))
        }
        None if verb == "create" => Some(Bucket::Todo.as_str().to_string()),
        None => None,
    };
    if let Some(name) = named {
        let stage = oxplow_domain::tasks::resolve_stage(&provider.stages, &name)
            .map_err(|m| invalid_at(&format!("/{field}"), format!("{}: {m}", provider.id)))?;
        fields.insert(field.into(), Value::String(stage.id.clone()));
    }
    Ok(())
}

/// The item a verb wrote: a create's from its result, else the input's
/// `ref`.
fn written_ref(verb: &str, input: &Value, result: &Value) -> Option<String> {
    let source = if verb == "create" { result } else { input };
    source["uri"].as_str().map(str::to_string)
}

/// The stage a state verb put its item in, from the item its answer
/// recorded; a create its answer recorded no item for is in the stage it
/// asked for (resolved already). Whether that is a move is the caller's
/// question: the stage it was in before ([`prior_of`]) says.
fn recorded_stage(
    verb: &str,
    input: &Value,
    result: &Value,
    events: &[Envelope],
) -> Option<String> {
    let names_stage = match verb {
        "create" | "transition" => true,
        "update" => input.get("stage").is_some(),
        _ => false,
    };
    if !names_stage {
        return None;
    }
    let item_ref = written_ref(verb, input, result)?;
    let recorded = events
        .iter()
        .filter(|e| e.event_type == TaskRecorded::TYPE)
        .filter_map(|e| {
            serde_json::from_value::<oxplow_domain::events::schema::TaskRecordedV3>(
                e.payload.clone(),
            )
            .ok()
        })
        .find(|r| r.item.uri == item_ref)
        .map(|r| r.item.stage);
    match verb {
        "create" => recorded.or_else(|| input["stage"].as_str().map(str::to_string)),
        _ => recorded,
    }
}

/// The interface's events for what `verb` did, logged by core the same
/// way for every list (`.context/tasks.md`): from the verb, its
/// input, the list's answer and `state` — the state the verb put the item
/// in, when it changed (always, for a create).
///
/// - `create`: `task.created` and `task.state_changed`.
/// - `update`: `task.edited` naming the fields the input set
///   (`title`, `body`, `parent`, `native.<name>`), and
///   `task.state_changed` when the state moved.
/// - `transition`: `task.state_changed` when the bucket moved, else
///   `task.edited` naming `stage` when it moved between two of the
///   bucket's stages (as an update's does).
/// - `link`, `comment`, `delete`: `task.linked`, `.commented`
///   (naming the answer's `comment`, when it gives one), `.deleted`.
fn canonical_events(
    source: &str,
    verb: &str,
    input: &Value,
    result: &Value,
    state: Option<Bucket>,
    restaged: bool,
) -> Vec<Envelope> {
    let Some(item_ref) = written_ref(verb, input, result) else {
        return Vec::new();
    };
    let about = |env: Envelope| env.with_subject([item_ref.clone()]);
    let edited = |fields: Vec<String>| {
        about(Envelope::typed::<TaskEdited>(
            source,
            &TaskEditedV2 {
                task: item_ref.clone(),
                fields,
            },
        ))
    };
    let mut events = Vec::new();
    match verb {
        "create" => {
            if let Some(state) = state {
                events.push(about(Envelope::typed::<TaskCreated>(
                    source,
                    &TaskCreatedV2 {
                        task: item_ref.clone(),
                        state,
                    },
                )));
            }
        }
        "update" => {
            let mut fields: Vec<String> = [
                ("title", "title"),
                ("body", "body"),
                ("parent_uri", "parent"),
            ]
            .into_iter()
            .filter(|(key, _)| input.get(key).is_some())
            .map(|(_, field)| field.to_string())
            .collect();
            if let Some(Value::Object(native)) = input.get("native") {
                fields.extend(native.keys().map(|k| format!("native.{k}")));
            }
            if restaged {
                fields.push("stage".into());
            }
            if !fields.is_empty() {
                events.push(edited(fields));
            }
        }
        "transition" if restaged => events.push(edited(vec!["stage".into()])),
        // A link already there, or none to remove, changed nothing.
        "link" if result["linked"] != json!(false) => {
            let target = input["target"].as_str().unwrap_or_default().to_string();
            events.push(
                Envelope::typed::<TaskLinked>(
                    source,
                    &TaskLinkedV2 {
                        task: item_ref.clone(),
                        target: target.clone(),
                        link_type: input["link_type"].as_str().unwrap_or_default().into(),
                    },
                )
                .with_subject([item_ref.clone(), target]),
            );
        }
        "unlink" if result["unlinked"] != json!(false) => {
            let target = input["target"].as_str().unwrap_or_default().to_string();
            events.push(
                Envelope::typed::<TaskUnlinked>(
                    source,
                    &TaskUnlinkedV1 {
                        task: item_ref.clone(),
                        target: target.clone(),
                        link_type: input["link_type"].as_str().unwrap_or_default().into(),
                    },
                )
                .with_subject([item_ref.clone(), target]),
            );
        }
        "comment" => events.push(about(Envelope::typed::<TaskCommented>(
            source,
            &TaskCommentedV2 {
                task: item_ref.clone(),
                comment: result["comment"].as_str().map(str::to_string),
            },
        ))),
        "delete" => events.push(about(Envelope::typed::<TaskDeleted>(
            source,
            &TaskDeletedV2 {
                task: item_ref.clone(),
            },
        ))),
        _ => {}
    }
    if let Some(to) = state {
        events.push(about(Envelope::typed::<TaskStateChanged>(
            source,
            &TaskStateChangedV1 {
                task: item_ref.clone(),
                to,
            },
        )));
    }
    events
}

fn spec_name(verb: &str) -> String {
    format!("oxplow.task.{verb}")
}

/// What a `task_tracker.write` operation is beyond its handler: the input
/// its handler reads, and whether it returns an inverse.
pub struct Shape {
    pub schema: Value,
    pub undoable: bool,
}

// ---- oxplow.task.transition ----

pub const NAME: &str = "oxplow.task.transition";

pub fn transition_shape() -> Shape {
    Shape {
        schema: schema::<TaskTransitionInput>(),
        undoable: true,
    }
}

pub fn transition_op(registry: TaskTrackers, reads: oxplow_db::Database) -> Op {
    dispatching(
        transition_shape(),
        registry,
        "transition",
        ref_target,
        None,
        Some(reads),
    )
}

// ---- oxplow.task.create ----

pub const CREATE: &str = "oxplow.task.create";

pub fn create_shape() -> Shape {
    Shape {
        schema: schema::<TaskCreateInput>(),
        // Undoing a filing would be deleting an item — not what undo is for.
        undoable: false,
    }
}

pub fn create_op(registry: TaskTrackers, links: LinkDeps) -> Op {
    dispatching(
        create_shape(),
        registry,
        "create",
        create_target,
        Some(links.clone()),
        Some(links.db),
    )
}

// ---- oxplow.task.update ----

pub const UPDATE: &str = "oxplow.task.update";

pub fn update_shape() -> Shape {
    Shape {
        schema: schema::<TaskUpdateInput>(),
        undoable: true,
    }
}

pub fn update_op(registry: TaskTrackers, links: LinkDeps) -> Op {
    let reads = links.db.clone();
    dispatching(
        update_shape(),
        registry,
        "update",
        update_target,
        Some(links),
        Some(reads),
    )
}

// ---- oxplow.task.link ----

pub const LINK: &str = "oxplow.task.link";
pub const UNLINK: &str = "oxplow.task.unlink";

/// `link` and `unlink` undo each other.
pub fn link_op(registry: TaskTrackers, links: LinkDeps) -> Op {
    let shape = Shape {
        schema: schema::<TaskLinkInput>(),
        undoable: true,
    };
    dispatching(shape, registry, "link", link_target, Some(links), None)
}

pub fn unlink_op(registry: TaskTrackers, links: LinkDeps) -> Op {
    let shape = Shape {
        schema: schema::<TaskLinkInput>(),
        undoable: true,
    };
    dispatching(shape, registry, "unlink", link_target, Some(links), None)
}

// ---- oxplow.task.comment ----

pub const COMMENT: &str = "oxplow.task.comment";

pub fn comment_op(registry: TaskTrackers) -> Op {
    let shape = Shape {
        schema: schema::<TaskCommentInput>(),
        undoable: false,
    };
    dispatching(shape, registry, "comment", comment_target, None, None)
}

// ---- oxplow.task.delete ----

pub const DELETE: &str = "oxplow.task.delete";

pub fn delete_op(registry: TaskTrackers) -> Op {
    let shape = Shape {
        schema: schema::<TaskDeleteInput>(),
        undoable: false,
    };
    dispatching(shape, registry, "delete", delete_target, None, None)
        .confirm_at_least(oxplow_domain::Confirm::Destructive)
}

// ---- oxplow.task.assign / oxplow.task.reorder: core's assignment ----

pub const ASSIGN: &str = "oxplow.task.assign";
pub const REORDER: &str = "oxplow.task.reorder";

/// Next to which item of a queue an item goes (neither: its end).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Before(String),
    After(String),
}

fn place_of(before: Option<String>, after: Option<String>) -> Result<Option<Place>, CommandError> {
    match (before, after) {
        (Some(_), Some(_)) => Err(invalid_at(
            "/after",
            "give `before` or `after`, not both".into(),
        )),
        (Some(b), None) => Ok(Some(Place::Before(b))),
        (None, Some(a)) => Ok(Some(Place::After(a))),
        (None, None) => Ok(None),
    }
}

fn place_fields(place: &Option<Place>) -> (Option<String>, Option<String>) {
    match place {
        Some(Place::Before(b)) => (Some(b.clone()), None),
        Some(Place::After(a)) => (None, Some(a.clone())),
        None => (None, None),
    }
}

/// `task.assigned` for `item`: from one assignee to another (`None`:
/// nobody), next to an item of `to`'s queue.
fn assigned(
    source: &str,
    item: &str,
    from: Option<String>,
    to: Option<String>,
    place: Option<Place>,
) -> Envelope {
    let (before, after) = place_fields(&place);
    Envelope::typed::<TaskAssigned>(
        source,
        &TaskAssignedV1 {
            task: item.to_string(),
            from,
            to,
            before,
            after,
        },
    )
    .with_subject([item.to_string()])
}

/// Why `to` can't be given work, if it can't: a thread must exist and be
/// open (a closed thread's work went back to nobody). The person always
/// can.
fn assignee_problem(
    conn: &rusqlite::Connection,
    to: &Assignee,
) -> Result<Option<String>, oxplow_domain::DomainError> {
    use rusqlite::OptionalExtension as _;
    let Assignee::Thread(thread) = to else {
        return Ok(None);
    };
    let closed: Option<bool> = conn
        .query_row(
            "SELECT closed_at IS NOT NULL OR archived_at IS NOT NULL FROM threads WHERE id = ?1",
            [thread.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(oxplow_db::map_sql_err)?;
    Ok(match closed {
        None => Some(format!("no thread `{thread}`")),
        Some(true) => Some(format!("thread `{thread}` is closed")),
        Some(false) => None,
    })
}

/// An assignee's queue as the interface shows it: the open items assigned
/// to `to` (`None`: nobody's), in order.
fn queue_tx(conn: &rusqlite::Connection, to: Option<&str>) -> Result<Vec<String>, CommandError> {
    let mut stmt = conn
        .prepare(
            "SELECT uri FROM v_task
             WHERE assignee IS ?1 AND state NOT IN ('done', 'canceled')
             ORDER BY rank IS NULL, rank, created_at, uri",
        )
        .map_err(oxplow_db::map_sql_err)?;
    let rows = stmt
        .query_map([to], |r| r.get::<_, String>(0))
        .map_err(oxplow_db::map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))
}

/// Where `item` sits in `queue`: next to the item before it, else the one
/// after it; `None` when it's alone there or not in it.
fn place_in(queue: &[String], item: &str) -> Option<Place> {
    let at = queue.iter().position(|u| u == item)?;
    if at > 0 {
        Some(Place::After(queue[at - 1].clone()))
    } else {
        queue.get(at + 1).map(|next| Place::Before(next.clone()))
    }
}

/// `queue` with `item` put at `place` (its end without one).
fn placed(queue: &[String], item: &str, place: &Option<Place>) -> Vec<String> {
    let mut order: Vec<String> = queue.iter().filter(|u| *u != item).cloned().collect();
    let at = match place {
        Some(Place::Before(b)) => order.iter().position(|u| u == b).unwrap_or(order.len()),
        Some(Place::After(a)) => order
            .iter()
            .position(|u| u == a)
            .map_or(order.len(), |i| i + 1),
        None => order.len(),
    };
    order.insert(at, item.to_string());
    order
}

/// Assign `uri` to `to` — `None` keeps whom it's assigned to (a reorder)
/// — at `place` in that queue: the run's result, its inverse and its
/// `task.assigned`, or `unchanged` when nothing moves.
fn place_item(
    ctx: &TxCtx<'_>,
    uri: String,
    to: Option<Option<Assignee>>,
    place: Option<Place>,
    reorder: bool,
) -> Result<HandlerOutput, CommandError> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(String, Option<String>)> = ctx
        .conn
        .query_row(
            "SELECT state, assignee FROM v_task WHERE uri = ?1",
            [&uri],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(oxplow_db::map_sql_err)?;
    let Some((state, from)) = row else {
        return Err(invalid_at("/uri", format!("no task `{uri}`")));
    };
    let to = match to {
        None => from.clone(),
        Some(None) => None,
        Some(Some(assignee)) => {
            if let Some(problem) = assignee_problem(ctx.conn, &assignee)? {
                return Err(invalid_at("/to", problem));
            }
            Some(assignee.uri())
        }
    };
    let closed = matches!(state.as_str(), "done" | "canceled");
    if closed && (to.is_some() || place.is_some()) {
        return Err(invalid_at(
            "/uri",
            format!("`{uri}` is {state}: a closed item is nobody's to work on"),
        ));
    }
    let queue = queue_tx(ctx.conn, to.as_deref())?;
    if let Some(Place::Before(n) | Place::After(n)) = &place {
        let field = if matches!(place, Some(Place::Before(_))) {
            "/before"
        } else {
            "/after"
        };
        if *n == uri || !queue.contains(n) {
            return Err(invalid_at(
                field,
                format!(
                    "`{n}` isn't in {}'s queue",
                    to.as_deref().unwrap_or("nobody")
                ),
            ));
        }
    }
    let from_queue = queue_tx(ctx.conn, from.as_deref())?;
    let was = place_in(&from_queue, &uri);
    if to == from && placed(&queue, &uri, &place) == queue {
        return Ok(HandlerOutput {
            result: json!({ "uri": uri, "assignee": to }),
            unchanged: true,
            ..HandlerOutput::default()
        });
    }
    let (before, after) = place_fields(&was);
    let inverse = if reorder {
        CommandCall {
            name: REORDER.into(),
            input: json!({ "uri": uri, "before": before, "after": after }),
        }
    } else {
        CommandCall {
            name: ASSIGN.into(),
            input: json!({ "uri": uri, "to": from, "before": before, "after": after }),
        }
    };
    let source = ctx.actor().source();
    Ok(HandlerOutput {
        result: json!({ "uri": uri, "assignee": to }),
        inverse: Some(inverse),
        events: vec![assigned(&source, &uri, from, to, place)],
        ..HandlerOutput::default()
    })
}

/// With no list active (none, a sink) there is nothing to assign: the
/// run succeeds as every verb does then, `{ tracked: false }`, and
/// records nothing.
fn untracked(registry: &TaskTrackers, input: &Value) -> Option<HandlerOutput> {
    let active = registry.get(&registry.active()).ok()?;
    active.sink.then(|| HandlerOutput {
        result: json!({ "uri": input["uri"], "tracked": false }),
        unchanged: true,
        ..HandlerOutput::default()
    })
}

/// `oxplow.task.assign { uri, to?, before?, after? }`: who is going to
/// work on an item — a thread, the person, or nobody — at the end of that
/// queue or next to an item there. Core's, for every list; a closed item
/// is refused (nobody's to work on), as is a closed thread. Undone by
/// assigning it back where it was.
pub fn assign_op(registry: TaskTrackers) -> Op {
    Op::new(
        "task_tracker.write",
        "assign",
        schema::<TaskAssignInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input: Value| {
            if let Some(untracked) = untracked(&registry, &input) {
                return Ok(untracked);
            }
            let input: TaskAssignInput = parse(with_loose_refs(&registry, input)?)?;
            let to = input
                .to
                .as_deref()
                .map(|raw| Assignee::parse(raw).map_err(|m| invalid_at("/to", m)))
                .transpose()?;
            let place = place_of(input.before, input.after)?;
            place_item(ctx, input.uri, Some(to), place, false)
        })),
    )
}

/// `oxplow.task.reorder { uri, before?, after? }`: an item's place in
/// its assignee's queue (or among the unassigned). Core's, for every list.
pub fn reorder_op(registry: TaskTrackers) -> Op {
    Op::new(
        "task_tracker.write",
        "reorder",
        schema::<TaskReorderInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input: Value| {
            if let Some(untracked) = untracked(&registry, &input) {
                return Ok(untracked);
            }
            let input: TaskReorderInput = parse(with_loose_refs(&registry, input)?)?;
            let place = place_of(input.before, input.after)?;
            place_item(ctx, input.uri, None, place, true)
        })),
    )
}

/// `task.assigned` events releasing every open item assigned to
/// `assignee` (a thread that closed): its work goes back to nobody.
pub fn release_all_tx(
    conn: &rusqlite::Connection,
    source: &str,
    assignee: &str,
) -> Result<Vec<Envelope>, CommandError> {
    Ok(queue_tx(conn, Some(assignee))?
        .into_iter()
        .map(|item| assigned(source, &item, Some(assignee.to_string()), None, None))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The task a run's result names, as its store holds it: the item's
    /// own fields, which the interface's answer (`{ ref, state? }`) leaves
    /// out.
    async fn task_row(svc: &crate::Services, result: &Value) -> Value {
        use oxplow_oxtask::OxtaskStore as _;
        let id = oxplow_oxtask::oxtask_of_task_ref(result["uri"].as_str().unwrap()).unwrap();
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
    use oxplow_domain::Actor;
    use oxplow_domain::StreamId;
    use oxplow_oxtask::OxtaskId;
    use oxplow_oxtask::{task_ref, OxtaskPriority, OxtaskStatus};

    /// tsk775: filing or editing an oxplow task answers with the
    /// `[[…]]` links in its body that don't resolve, as a note does, so an
    /// agent fixes a broken link in the same turn.
    #[tokio::test]
    async fn a_task_body_answers_with_its_broken_links() {
        let fx = crate::test_fixtures::services_with_thread().await;
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
        let item = filed.result["uri"].as_str().unwrap().to_string();
        let edited = run(
            UPDATE,
            json!({ "uri": item, "body": "now see [[tsk99998]]" }),
        )
        .await;
        assert_eq!(targets(&edited.result), vec!["tsk99998"]);
        let fixed = run(UPDATE, json!({ "uri": item, "body": "no links" })).await;
        assert_eq!(targets(&fixed.result), Vec::<String>::new());
    }

    /// A blank title is refused at `/title` on create and update, before any
    /// list sees it; a title is kept when it isn't touched.
    #[tokio::test]
    async fn a_blank_title_is_refused_at_the_title() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let run = |name: &'static str, input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&Actor::Human, name, input, false).await }
        };
        let made = run(CREATE, json!({ "title": "Real" })).await.unwrap();
        let item = made.result["uri"].as_str().unwrap().to_string();
        for (name, input) in [
            (CREATE, json!({ "title": "  " })),
            (UPDATE, json!({ "uri": item, "title": "" })),
        ] {
            match run(name, input).await.unwrap_err() {
                CommandError::Invalid { field, .. } => {
                    assert_eq!(field.as_deref(), Some("/title"), "{name}");
                }
                other => panic!("{name}: expected Invalid, got {other:?}"),
            }
        }
        run(UPDATE, json!({ "uri": item, "body": "title untouched" }))
            .await
            .unwrap();
    }

    /// P5.C2: the commands take canonical refs and refuse another
    /// provider's, naming the registered ones.
    #[tokio::test]
    async fn another_lists_ref_is_refused_naming_the_active_list() {
        let fx = crate::test_fixtures::services_with_thread().await;
        for (name, input) in [
            (
                NAME,
                json!({ "uri": "oxplow:oxplow/task?id=ENG-12&provider=issues", "to": "done" }),
            ),
            (
                UPDATE,
                json!({ "uri": "oxplow:oxplow/task?id=ENG-12&provider=issues", "title": "x" }),
            ),
            (
                COMMENT,
                json!({ "uri": "oxplow:oxplow/task?id=ENG-12&provider=issues", "body": "x" }),
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
                    assert_eq!(field.as_deref(), Some("/uri"), "{name}");
                    assert_eq!(
                        message,
                        "`oxplow:oxplow/task?id=ENG-12&provider=issues` is issues's, which isn't the active task tracker (`oxtask`)",
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
                json!({ "uri": "ENG-12", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("oxplow:oxplow/task?id=<id>&provider=<provider>"),
            "{err}"
        );
    }

    /// `transition` names one of the list's stages or a bucket for its
    /// default; anything else is refused at `/to` naming the stages; the
    /// inverse names the stage it left, so undo restores it. A move between
    /// two stages of one bucket isn't a state change.
    #[tokio::test]
    async fn a_transition_takes_a_stage_or_a_bucket() {
        let fx = crate::test_fixtures::services_with_task().await;
        let t = task_ref(fx.task);
        let run = |input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&Actor::Human, NAME, input, false).await }
        };
        let err = run(json!({ "uri": t, "to": "shipped" })).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/to" && message.contains("backlog, ready, in_progress, blocked, done, canceled")),
            "{err:?}"
        );
        // A bucket lands on its default stage: oxplow's `todo` is `ready`.
        let out = run(json!({ "uri": t, "to": "todo" })).await.unwrap();
        assert_eq!(out.result["stage"], "ready");
        assert_eq!(out.result["state"], "todo");
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "ready");
        let inverse = out.inverse.clone().unwrap();
        assert_eq!(inverse.name, NAME);
        assert_eq!(inverse.input, json!({ "uri": t, "to": "in_progress" }));
        let row = fx
            .svc
            .sql
            .query_sql(
                "SELECT state, stage, stage_title FROM v_task WHERE uri = ?1",
                vec![oxplow_db::SqlCell::Text(t.clone())],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&row.rows[0]).unwrap(),
            json!(["todo", "ready", "Ready"])
        );
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_oxtask::OxtaskStore as _;
        let back = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(back.status, OxtaskStatus::InProgress);
    }

    /// P7.A1: the input is the `v_task` shape — oxplow's own fields
    /// ride under `native` and nothing else does.
    #[tokio::test]
    async fn a_create_keeps_oxplows_fields_under_native() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({
                    "title": "filed",
                    "body": "the body",
                    "assignee": fx.thread.to_string(),
                    "native": { "priority": "high" },
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(task_row(&fx.svc, &out.result).await["priority"], "high");
        assert_eq!(
            assignee(&fx, &out.result).await,
            Some(oxplow_domain::uri::build::thread_uri(fx.thread)),
            "who it's for is core's, not the task's"
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
            (
                json!({ "title": "x", "assignee": "nope" }),
                Some("/assignee"),
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

    /// tsk1000: a comment is recorded as its author — an effect's or
    /// oxplow's own isn't shown as the person's.
    #[tokio::test]
    async fn a_comment_is_recorded_as_whoever_made_it() {
        let fx = crate::test_fixtures::services_with_task().await;
        let item = oxplow_oxtask::task_ref(fx.task);
        for (actor, author) in [
            (Actor::Human, "user"),
            (
                Actor::Effect {
                    effect: "acme/notify".into(),
                    thread_id: None,
                    stream_id: None,
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
                    json!({ "uri": item, "body": "noted" }),
                    false,
                )
                .await
                .unwrap();
            let note = commented.result["comment"].as_str().unwrap();
            let id = note.trim_start_matches("not").to_string();
            assert_eq!(
                column(&fx.svc, "SELECT author FROM oxtask_note WHERE id = ?1", id).await,
                author,
                "{actor:?}"
            );
        }
    }

    /// The active task tracker is resolved from the config as it is now — a
    /// `create` right after a person chose another files there, with no
    /// reconcile in between; one that isn't available resolves to none, and
    /// filing says so.
    /// A loose id the active task tracker declares (`tsk12` for oxplow's
    /// tasks) is that list's item, in every task ref field; one it doesn't
    /// declare is refused naming the shapes.
    #[tokio::test]
    async fn a_loose_id_resolves_against_the_active_task_tracker() {
        let fx = crate::test_fixtures::services_with_task().await;
        let loose = fx.task.to_string();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "uri": loose, "to": "blocked" }),
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
                json!({ "title": "child", "parent_uri": loose }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &child.result).await["parent_id"],
            json!(loose)
        );
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "uri": "ENG-1", "to": "done" }),
                false,
            )
            .await
            .unwrap_err();
        match err {
            CommandError::Invalid { field, message } => {
                assert_eq!(field.as_deref(), Some("/uri"));
                assert!(message.contains("tsk"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_create_reads_the_active_provider_the_config_names_now() {
        let fx = crate::test_fixtures::services_with_thread().await;
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("task_tracker".into(), "issues".into());
        crate::test_fixtures::activate(&fx.svc).await;
        // The chosen one isn't running: none is, and files nowhere.
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "where?" }), false)
            .await
            .unwrap();
        assert_eq!(out.result["tracked"], json!(false));
        assert_eq!(fx.svc.tasks.active(), oxplow_domain::capability::NONE);
    }

    /// P7.A2, tsk1058: every `create` files on the active tracker — oxplow's
    /// by default; one that isn't running is an error naming it, never a
    /// fallback — and no caller can name another.
    #[tokio::test]
    async fn every_create_files_on_the_active_tracker() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let filed = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "here" }), false)
            .await
            .unwrap();
        assert!(filed.result["uri"]
            .as_str()
            .unwrap()
            .ends_with("&provider=oxtask"));
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
            .insert("task_tracker".into(), "tracker".into());
        crate::test_fixtures::activate(&fx.svc).await;
        let before = queue(&fx, None).await.len();
        // Never another list: the chosen one isn't running, so none files
        // it nowhere.
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "where?" }), false)
            .await
            .unwrap();
        assert_eq!(out.result["tracked"], json!(false));
        assert_eq!(queue(&fx, None).await.len(), before, "nothing filed");
    }

    /// Filing assigns nobody, whoever files — an agent's item is in the
    /// pool, not on its thread — unless the create names an assignee, which
    /// is core's record (the list never sees it) and must be an open thread
    /// or the person.
    #[tokio::test]
    async fn a_create_assigns_nobody_unless_it_names_an_assignee() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let create = |actor: Actor, input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&actor, CREATE, input, false).await }
        };
        let mine = create(agent, json!({ "title": "mine" })).await.unwrap();
        assert_eq!(assignee(&fx, &mine.result).await, None);
        let theirs = create(
            Actor::Human,
            json!({ "title": "theirs", "assignee": fx.thread.to_string() }),
        )
        .await
        .unwrap();
        assert_eq!(
            assignee(&fx, &theirs.result).await,
            Some(oxplow_domain::uri::build::thread_uri(fx.thread))
        );
        let me = create(
            Actor::Human,
            json!({ "title": "mine to do", "assignee": "oxplow:oxplow/person" }),
        )
        .await
        .unwrap();
        assert_eq!(
            assignee(&fx, &me.result).await,
            Some("oxplow:oxplow/person".into())
        );
        for input in [
            json!({ "title": "nowhere", "assignee": "thr999" }),
            json!({ "title": "not one", "assignee": "tsk3" }),
            json!({ "title": "old", "native": { "thread": fx.thread.to_string() } }),
        ] {
            let err = create(Actor::Human, input.clone()).await.unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
    }

    /// An agent that starts a task takes it: the start assigns it to the
    /// agent's thread, from whoever had it. A person starting one assigns
    /// nothing, and an agent's other moves leave the assignee alone.
    #[tokio::test]
    async fn an_agent_that_starts_a_task_takes_it() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let run = |actor: Actor, name: &'static str, input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&actor, name, input, false).await.unwrap() }
        };
        let mine = oxplow_domain::uri::build::thread_uri(fx.thread);
        let pooled = run(Actor::Human, CREATE, json!({ "title": "pooled" })).await;
        let item = pooled.result["uri"].as_str().unwrap().to_string();
        run(agent.clone(), NAME, json!({ "uri": item, "to": "todo" })).await;
        assert_eq!(assignee(&fx, &pooled.result).await, None);
        run(
            agent.clone(),
            NAME,
            json!({ "uri": item, "to": "in_progress" }),
        )
        .await;
        assert_eq!(assignee(&fx, &pooled.result).await, Some(mine.clone()));
        let persons = run(
            Actor::Human,
            CREATE,
            json!({ "title": "the person's", "assignee": "oxplow:oxplow/person" }),
        )
        .await;
        let item = persons.result["uri"].as_str().unwrap().to_string();
        run(
            Actor::Human,
            NAME,
            json!({ "uri": item, "to": "in_progress" }),
        )
        .await;
        assert_eq!(
            assignee(&fx, &persons.result).await.as_deref(),
            Some("oxplow:oxplow/person")
        );
        run(agent, NAME, json!({ "uri": item, "to": "todo" })).await;
        run(Actor::Human, NAME, json!({ "uri": item, "to": "todo" })).await;
        assert_eq!(
            assignee(&fx, &persons.result).await.as_deref(),
            Some("oxplow:oxplow/person")
        );
    }

    /// Who the item a run's result names is assigned to.
    async fn assignee(fx: &crate::test_fixtures::ThreadFixture, result: &Value) -> Option<String> {
        let item = result["uri"].as_str().unwrap().to_string();
        fx.svc
            .db
            .read(move |tx| {
                tx.query_row("SELECT assignee FROM v_task WHERE uri = ?1", [item], |r| {
                    r.get(0)
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A link belongs to its task: a person links two backlog tasks, and an
    /// unknown link type is refused.
    #[tokio::test]
    async fn a_persons_link_belongs_to_its_task() {
        let fx = crate::test_fixtures::services_with_task().await;
        let from = task_ref(fx.task);
        let other = file_for(&fx, "other", None).await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "uri": from, "target": other, "link_type": "blocks" }),
                false,
            )
            .await
            .unwrap();
        let another = file_for(&fx, "another", None).await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "uri": other, "target": another, "link_type": "relates_to" }),
                false,
            )
            .await
            .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                LINK,
                json!({ "uri": from, "target": other, "link_type": "caused_by" }),
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
        use oxplow_oxtask::OxtaskStore as _;
        let fx = crate::test_fixtures::services_with_task().await;
        let other = fx
            .svc
            .commands
            .run(&Actor::Human, CREATE, json!({ "title": "other" }), false)
            .await
            .unwrap()
            .result["uri"]
            .as_str()
            .unwrap()
            .to_string();
        fx.svc.task_store.soft_delete(fx.task).await.unwrap();
        let gone = task_ref(fx.task);
        for (name, input) in [
            (COMMENT, json!({ "uri": gone, "body": "hello" })),
            (
                LINK,
                json!({ "uri": other, "target": gone, "link_type": "blocks" }),
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(
                    &Actor::Agent {
                        session_id: None,
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

    /// `oxplow.task.link` and `oxplow.task.comment` write the link and the
    /// note, each with its event, caused by the run.
    /// A link may point at any ref: a task says what it is `about`
    /// (a collector), and the link reads back with its target's kind, as
    /// an edge of the target's backlinks, and as the item's record. It is
    /// undone by `unlink`, which undoes back to the link; a target of an
    /// unregistered kind is refused, another list's item still is. The
    /// target needn't exist: a ref may dangle.
    #[tokio::test]
    async fn an_item_links_about_any_ref_and_unlinks() {
        let fx = crate::test_fixtures::services_with_task().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let item = task_ref(fx.task);
        let about = json!({ "uri": item, "target": "oxplow:oxplow/collector?collector=gone/hot", "link_type": "about" });
        let linked = fx
            .svc
            .commands
            .run(&agent, LINK, about.clone(), false)
            .await
            .unwrap();
        let link_rows = || async {
            fx.svc
                .sql
                .query_sql(
                    "SELECT to_uri, to_kind, link_type FROM v_task_link WHERE from_uri = ?1",
                    vec![oxplow_db::SqlCell::Text(item.clone())],
                    None,
                )
                .await
                .unwrap()
                .rows
        };
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            serde_json::to_value(link_rows().await).unwrap(),
            json!([[
                "oxplow:oxplow/collector?collector=gone/hot",
                "oxplow/collector",
                "about"
            ]])
        );
        let backlinks = fx
            .svc
            .page_ref_store
            .list_backlinks(
                &oxplow_domain::uri::build::collector_uri("gone", "hot"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            backlinks.len(),
            1,
            "the collector's backlinks name the item"
        );

        // Undo is an unlink; undoing that links again.
        let undone = fx
            .svc
            .commands
            .undo(&Actor::Human, linked.audit_id.unwrap(), false)
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        assert!(link_rows().await.is_empty());
        fx.svc
            .commands
            .undo(&Actor::Human, undone.audit_id.unwrap(), false)
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        assert_eq!(link_rows().await.len(), 1);
        fx.svc
            .commands
            .run(&agent, UNLINK, about, false)
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        assert!(link_rows().await.is_empty());
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "task.unlinked"));

        for (target, why) in [
            ("nope:x", "an unregistered kind"),
            (
                "oxplow:oxplow/task?id=W-1&provider=fake",
                "another list's item",
            ),
            ("collector", "not a ref"),
        ] {
            let err = fx
                .svc
                .commands
                .run(
                    &agent,
                    LINK,
                    json!({ "uri": item, "target": target, "link_type": "about" }),
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field, .. } if field.as_deref() == Some("/target")),
                "{why}: {err:?}"
            );
        }
    }

    /// An item is filed with its links, in one change.
    #[tokio::test]
    async fn an_item_is_created_with_its_links() {
        let fx = crate::test_fixtures::services_with_task().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let created = fx
            .svc
            .commands
            .run(
                &agent,
                CREATE,
                json!({
                    "title": "Repair collector:triage/hot",
                    "links": [{ "target": "oxplow:oxplow/collector?collector=triage/hot", "link_type": "about" }],
                }),
                false,
            )
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        let item = created.result["uri"].as_str().unwrap().to_string();
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT to_uri, link_type FROM v_task_link WHERE from_uri = ?1",
                vec![oxplow_db::SqlCell::Text(item)],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            json!([["oxplow:oxplow/collector?collector=triage/hot", "about"]])
        );
    }

    #[tokio::test]
    async fn links_and_comments_are_commands_with_their_events() {
        let fx = crate::test_fixtures::services_with_task().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let from = task_ref(fx.task);
        let other = fx
            .svc
            .commands
            .run(&agent, CREATE, json!({ "title": "other" }), false)
            .await
            .unwrap()
            .result["uri"]
            .as_str()
            .unwrap()
            .to_string();
        let linked = fx
            .svc
            .commands
            .run(
                &agent,
                LINK,
                json!({ "uri": from, "target": other, "link_type": "blocks" }),
                false,
            )
            .await
            .unwrap();
        let from_id = oxplow_domain::uri::build::task_of_uri(&from)
            .unwrap()
            .1
            .trim_start_matches("tsk")
            .to_string();
        assert_eq!(
            column(
                &fx.svc,
                "SELECT link_type FROM oxtask_link WHERE from_item_id = ?1",
                from_id
            )
            .await,
            "blocks"
        );
        let commented = fx
            .svc
            .commands
            .run(
                &agent,
                COMMENT,
                json!({ "uri": from, "body": "see [[src/lib.rs]]" }),
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
                "SELECT author FROM oxtask_note WHERE id = ?1",
                note.trim_start_matches("not").to_string()
            )
            .await,
            "agent"
        );
        let logged = events
            .iter()
            .find(|e| {
                e.envelope.cause == commented.event_id && e.envelope.event_type == "task.commented"
            })
            .unwrap();
        assert_eq!(
            logged.envelope.payload,
            json!({ "task": from, "comment": note })
        );
        for (out, event_type) in [(&linked, "task.linked"), (&commented, "task.commented")] {
            let caused: Vec<&str> = events
                .iter()
                .filter(|e| e.envelope.cause == out.event_id)
                .map(|e| e.envelope.event_type.as_str())
                .collect();
            assert_eq!(caused, vec!["task.recorded", event_type]);
        }
        // Both are the item's page refs once the pump restates it from
        // the interface.
        fx.svc.event_pump.run_once().await.unwrap();
        let id = from.clone();
        let edges: Vec<(String, String)> = fx
            .svc
            .db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT ref_type, target_uri FROM page_ref
                         WHERE source_uri = ?1 ORDER BY ref_type",
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
        let other_id = other.as_str();
        assert!(
            edges.contains(&(
                "comment_file_ref".into(),
                oxplow_domain::uri::build::file_uri("src/lib.rs")
            )),
            "{edges:?}"
        );
        assert!(
            edges.contains(&("task_link:blocks".into(), other_id.into())),
            "{edges:?}"
        );
        let err = fx
            .svc
            .commands
            .run(&agent, COMMENT, json!({ "uri": from, "body": "  " }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("body"), "{err}");
    }

    /// A transition is one transaction with its audit: the status and
    /// core's `task.state_changed` carry the actor's source and are
    /// caused by the run's `command.executed`.
    /// A transition to the state the item is in moves nothing, so it logs
    /// no `task.state_changed` — the event says the state moved.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_transition_to_the_state_it_is_in_logs_no_change() {
        let fx = crate::test_fixtures::services_with_task().await;
        let changes = |svc: Arc<crate::Services>| async move {
            svc.event_log_store
                .read_after(0, 1000)
                .await
                .unwrap()
                .iter()
                .filter(|e| e.envelope.event_type == "task.state_changed")
                .count()
        };
        let before = changes(fx.svc.clone()).await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "uri": task_ref(fx.task), "to": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(changes(fx.svc.clone()).await, before, "already in progress");
        fx.svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "uri": task_ref(fx.task), "to": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(changes(fx.svc.clone()).await, before + 1);
    }

    #[tokio::test]
    async fn a_transition_commits_with_its_audit_and_names_its_cause() {
        let fx = crate::test_fixtures::services_with_task().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let outcome = fx
            .svc
            .commands
            .run(
                &agent,
                NAME,
                json!({ "uri": task_ref(fx.task), "to": "done" }),
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
                // The list's own record, then core's events: done, it's
                // nobody's to work on.
                ("task.recorded", "provider:oxtask"),
                ("task.state_changed", source.as_str()),
                ("task.assigned", source.as_str()),
            ]
        );
        assert!(events
            .iter()
            .any(|e| e.envelope.id == executed && e.envelope.event_type == "command.executed"));
    }

    /// `oxplow.task.update`: fields and state commit together, audited and
    /// undoable — an undo restores both.
    #[tokio::test]
    async fn an_update_edits_fields_and_state_atomically_and_undoes() {
        let fx = crate::test_fixtures::services_with_task().await;
        let bus = &fx.svc.commands;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let out = bus
            .run(
                &agent,
                UPDATE,
                json!({
                    "uri": task_ref(fx.task), "title": "renamed", "stage": "blocked",
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
            .filter(|e| e.envelope.event_type != "task.recorded")
            .map(|e| (e.envelope.event_type.as_str(), &e.envelope.payload))
            .collect();
        let item = task_ref(fx.task);
        assert_eq!(
            caused,
            vec![
                (
                    "task.edited",
                    &json!({ "task": item, "fields": ["title", "native.priority"] })
                ),
                (
                    "task.state_changed",
                    &json!({ "task": item, "to": "blocked" })
                ),
            ]
        );

        bus.undo(&agent, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        use oxplow_oxtask::OxtaskStore as _;
        let back = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(back.title, "t");
        assert_eq!(back.status, OxtaskStatus::InProgress);
        assert_eq!(back.priority, OxtaskPriority::Medium);
        let err = bus
            .run(
                &agent,
                UPDATE,
                json!({ "uri": task_ref(fx.task), "native": { "thread": "thr2" } }),
                false,
            )
            .await
            .unwrap_err();
        // Who works on an item is `oxplow.task.assign`'s, not a native
        // field.
        assert!(
            matches!(&err, CommandError::Invalid { field, .. } if field.as_deref() == Some("/native")),
            "{err:?}"
        );
    }

    /// Another thread's agent may edit a task and finish it: task
    /// bookkeeping isn't a claim on the worktree.
    #[tokio::test]
    async fn another_threads_agent_edits_and_finishes_tasks() {
        let fx = crate::test_fixtures::services_with_task().await;
        let other = other_threads_agent(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &other,
                UPDATE,
                json!({ "uri": task_ref(fx.task), "title": "renamed", "stage": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(task_row(&fx.svc, &out.result).await["status"], "done");
        use oxplow_oxtask::OxtaskStore as _;
        let row = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(
            (row.title.as_str(), row.status),
            ("renamed", OxtaskStatus::Done)
        );
    }

    /// Any thread may start a task: a task's state is a record, not a
    /// claim on the worktree (only edits are guarded, by isolation).
    #[tokio::test]
    async fn another_threads_agent_starts_and_files_tasks_in_progress() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let other = other_threads_agent(&fx).await;
        let later = file_for(
            &fx,
            "later",
            Some(&oxplow_domain::uri::build::thread_uri(fx.thread)),
        )
        .await;
        fx.svc
            .commands
            .run(
                &other,
                NAME,
                json!({ "uri": later, "to": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &other,
                CREATE,
                json!({ "title": "mine now", "assignee": "thr9", "stage": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            task_row(&fx.svc, &out.result).await["status"],
            "in_progress"
        );
    }

    /// `oxplow.task.create`: filing a task is audited to the actor, filed
    /// straight into `in_progress`; the body's mentions are projected by the
    /// pump.
    #[tokio::test]
    async fn a_create_is_audited_and_its_mentions_are_projected() {
        let fx = crate::test_fixtures::services_with_task().await;
        let agent = Actor::Agent {
            session_id: None,
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
                    "stage": "in_progress",
                    "assignee": fx.thread.to_string(),
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
        let id: OxtaskId =
            oxplow_oxtask::oxtask_of_task_ref(out.result["uri"].as_str().unwrap()).unwrap();
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
                "task.recorded",
                "task.created",
                "task.state_changed",
                "task.assigned"
            ]
        );
        fx.svc.event_pump.run_once().await.unwrap();
        let out_refs = fx
            .svc
            .page_ref_store
            .list_outbound(&task_ref(id), None)
            .await
            .unwrap();
        assert!(!out_refs.is_empty(), "the body mention was projected");
    }

    /// A request links the tasks it worked on, each with its evidence: a
    /// prompt that named two tasks, a task command the agent ran in the turn,
    /// the task in progress for the thread when the turn ended. A person's
    /// command links nothing, whatever turn is open.
    #[tokio::test]
    async fn a_request_links_the_tasks_it_worked_on_with_their_evidence() {
        let fx = crate::test_fixtures::services_with_task().await;
        let svc = &fx.svc;
        let named = crate::test_fixtures::file_item(svc, json!({ "title": "named" })).await;
        let by_person = crate::test_fixtures::file_item(svc, json!({ "title": "person" })).await;
        let hook = |kind, prompt: Option<String>| crate::HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
            kind,
            thread_id: Some(fx.thread),
            stream_id: None,
            agent_session_id: Some(fx.session),
            session_id: None,
            payload_json: "{}".into(),
            prompt: prompt.map(Into::into),
            decision: None,
            tool: None,
            subagent: None,
            harness_turn: None,
        };
        let prompt = format!(
            "do {} and {}",
            oxplow_domain::uri::prompt_refs::uri_mention(&task_ref(fx.task)),
            oxplow_domain::uri::prompt_refs::uri_mention(&task_ref(named)),
        );
        svc.hook_ingest
            .ingest(hook(
                oxplow_domain::HookKind::UserPromptSubmit,
                Some(prompt),
            ))
            .await
            .unwrap();
        let agent = Actor::Agent {
            session_id: Some(fx.session),
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        svc.commands
            .run(
                &agent,
                UPDATE,
                json!({ "uri": task_ref(named), "title": "named!" }),
                false,
            )
            .await
            .unwrap();
        // Moving a task is its own evidence: the first task is finished.
        svc.commands
            .run(
                &agent,
                NAME,
                json!({ "uri": task_ref(fx.task), "to": "done" }),
                false,
            )
            .await
            .unwrap();
        // A person's command in the same open turn is in no request.
        svc.commands
            .run(
                &Actor::Human,
                UPDATE,
                json!({ "uri": task_ref(by_person), "title": "person!" }),
                false,
            )
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(oxplow_domain::HookKind::Stop, None))
            .await
            .unwrap();
        let rows: Vec<(String, String)> = svc
            .db
            .read(|c| {
                let mut stmt = c
                    .prepare("SELECT task_uri, basis FROM request_task ORDER BY task_uri, basis")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let (first, second) = (task_ref(fx.task), task_ref(named));
        let mut expected = vec![
            (first.clone(), "prompt".to_string()),
            (first, "transition".to_string()),
            (second.clone(), "command".to_string()),
            (second, "prompt".to_string()),
        ];
        expected.sort();
        assert_eq!(rows, expected);
        assert!(rows.iter().all(|(t, _)| *t != task_ref(by_person)));
    }

    /// An agent that files a task straight into progress, naming no one,
    /// takes it like a start; a person's, or one that names an assignee,
    /// does not.
    #[tokio::test]
    async fn an_agents_create_in_progress_is_taken_by_its_thread() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        };
        let mine = oxplow_domain::uri::build::thread_uri(fx.thread);
        let assignee_of = |out: &serde_json::Value| {
            let uri = out["uri"].as_str().unwrap().to_string();
            let svc = fx.svc.clone();
            async move {
                svc.db
                    .read(move |c| {
                        c.query_row("SELECT assignee FROM v_task WHERE uri = ?1", [uri], |r| {
                            r.get::<_, Option<String>>(0)
                        })
                        .map_err(oxplow_db::map_sql_err)
                    })
                    .await
                    .unwrap()
            }
        };
        let by_agent = fx
            .svc
            .commands
            .run(
                &agent,
                CREATE,
                json!({ "title": "mine", "stage": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            assignee_of(&by_agent.result).await.as_deref(),
            Some(mine.as_str())
        );
        let by_person = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "theirs", "stage": "in_progress" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(assignee_of(&by_person.result).await, None);
    }

    /// Thread 9, another open thread in stream 1, as its agent.
    async fn other_threads_agent(fx: &crate::test_fixtures::ThreadFixture) -> Actor {
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                       VALUES (9, 1, 'q',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(9)),
            stream_id: Some(StreamId::new(1)),
        }
    }

    /// An unknown task fails the run and writes nothing but the error audit.
    #[tokio::test]
    async fn a_failed_transition_logs_nothing() {
        let fx = crate::test_fixtures::services_with_thread().await;
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
                json!({ "uri": "oxplow:oxplow/task?id=tsk999&provider=oxtask", "to": "done" }),
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

    /// An assignee's queue as the interface reads it: the open items
    /// assigned to `assignee` (`None`: nobody), in order.
    async fn queue(
        fx: &crate::test_fixtures::ThreadFixture,
        assignee: Option<&str>,
    ) -> Vec<String> {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT uri FROM v_task
                 WHERE assignee IS ?1 AND state NOT IN ('done', 'canceled')
                 ORDER BY rank IS NULL, rank, created_at, uri",
                vec![match assignee {
                    Some(a) => oxplow_db::SqlCell::Text(a.into()),
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

    /// File an item as a person, assigned to `assignee` (`None`: nobody).
    async fn file_for(
        fx: &crate::test_fixtures::ThreadFixture,
        title: &str,
        assignee: Option<&str>,
    ) -> String {
        let mut input = json!({ "title": title });
        if let Some(a) = assignee {
            input["assignee"] = json!(a);
        }
        fx.svc
            .commands
            .run(&Actor::Human, CREATE, input, false)
            .await
            .unwrap()
            .result["uri"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// `parent_uri: ""` detaches an item from its parent, as the input
    /// documents; undo puts the parent back.
    #[tokio::test]
    async fn an_empty_parent_ref_detaches_and_undo_reattaches() {
        let fx = crate::test_fixtures::services_with_task().await;
        let parent = task_ref(fx.task);
        let child = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "child", "parent_uri": parent }),
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
                json!({ "uri": child.result["uri"], "parent_uri": "" }),
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

    /// `oxplow.task.reorder` places an item before or after another in
    /// its assignee's queue (core's, for every list); undo puts it back.
    #[tokio::test]
    async fn reorder_places_an_item_in_its_queue_and_undo_puts_it_back() {
        let fx = crate::test_fixtures::services_with_task().await;
        let mine = oxplow_domain::uri::build::thread_uri(fx.thread);
        let t = task_ref(fx.task);
        let a = file_for(&fx, "a", Some(&mine)).await;
        let b = file_for(&fx, "b", Some(&mine)).await;
        let c = file_for(&fx, "c", Some(&mine)).await;
        assert_eq!(
            queue(&fx, Some(&mine)).await,
            vec![t.clone(), a.clone(), b.clone(), c.clone()]
        );
        let run = |input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&Actor::Human, REORDER, input, false).await }
        };
        let out = run(json!({ "uri": c, "before": a })).await.unwrap();
        assert_eq!(
            queue(&fx, Some(&mine)).await,
            vec![t.clone(), c.clone(), a.clone(), b.clone()]
        );
        run(json!({ "uri": t, "after": b })).await.unwrap();
        assert_eq!(
            queue(&fx, Some(&mine)).await,
            vec![c.clone(), a.clone(), b.clone(), t.clone()]
        );
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            queue(&fx, Some(&mine)).await,
            vec![a.clone(), b.clone(), c.clone(), t.clone()]
        );
        // The anchor must be in the same queue, and there's at most one.
        let other = file_for(&fx, "elsewhere", None).await;
        for input in [
            json!({ "uri": a, "before": other }),
            json!({ "uri": a, "before": b, "after": c }),
        ] {
            let err = run(input.clone()).await.unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
    }

    /// `oxplow.task.assign` gives an item to a thread, the person or
    /// nobody (the end of that queue, or next to an item there) and logs
    /// `task.assigned`; undo gives it back where it was. A closed
    /// thread and a closed item are refused; closing an item releases it.
    #[tokio::test]
    async fn assign_gives_an_item_to_someone_and_undo_gives_it_back() {
        let fx = crate::test_fixtures::services_with_task().await;
        let mine = oxplow_domain::uri::build::thread_uri(fx.thread);
        let person = oxplow_domain::uri::build::person_uri(None);
        let t = task_ref(fx.task);
        let a = file_for(&fx, "a", Some(&mine)).await;
        let x = file_for(&fx, "x", Some(&person)).await;
        let y = file_for(&fx, "y", Some(&person)).await;
        let run = |input: Value| {
            let commands = fx.svc.commands.clone();
            async move { commands.run(&Actor::Human, ASSIGN, input, false).await }
        };
        let out = run(json!({ "uri": t, "to": person, "before": y }))
            .await
            .unwrap();
        assert_eq!(out.result["assignee"], json!(person));
        assert_eq!(
            queue(&fx, Some(&person)).await,
            vec![x.clone(), t.clone(), y.clone()]
        );
        assert_eq!(queue(&fx, Some(&mine)).await, vec![a.clone()]);
        let logged = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        assert!(logged
            .iter()
            .any(|e| e.envelope.event_type == "task.assigned"
                && e.envelope.payload["task"] == json!(t)
                && e.envelope.payload["to"] == json!(person)));

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(queue(&fx, Some(&mine)).await, vec![t.clone(), a.clone()]);
        assert_eq!(queue(&fx, Some(&person)).await, vec![x.clone(), y.clone()]);

        run(json!({ "uri": x, "to": null })).await.unwrap();
        assert_eq!(queue(&fx, None).await, vec![x.clone()]);
        let err = run(json!({ "uri": y, "to": "thr999" })).await.unwrap_err();
        assert!(err.to_string().contains("thr999"), "{err}");

        // Done, it's nobody's; and nobody's it stays.
        fx.svc
            .commands
            .run(
                &Actor::Human,
                NAME,
                json!({ "uri": y, "to": "done" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(queue(&fx, Some(&person)).await, Vec::<String>::new());
        assert_eq!(assignee(&fx, &json!({ "uri": y })).await, None);
        let err = run(json!({ "uri": y, "to": mine })).await.unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    /// `oxplow.task.delete` asks first, then removes the task, logged as
    /// caused by the run.
    #[tokio::test]
    async fn delete_asks_first_then_removes_the_task() {
        let fx = crate::test_fixtures::services_with_task().await;
        let t = task_ref(fx.task);
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "uri": t }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        let out = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "uri": t }), true)
            .await
            .unwrap();
        assert!(
            queue(&fx, Some(&oxplow_domain::uri::build::thread_uri(fx.thread)))
                .await
                .is_empty()
        );
        let executed = out.event_id.unwrap();
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let caused: Vec<&str> = events
            .iter()
            .filter(|e| e.envelope.cause.as_ref() == Some(&executed))
            .map(|e| e.envelope.event_type.as_str())
            .collect();
        assert_eq!(caused, vec!["task.recorded", "task.deleted"]);
        let again = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "uri": t }), true)
            .await
            .unwrap_err();
        assert!(matches!(again, CommandError::Invalid { .. }), "{again:?}");
    }
}
