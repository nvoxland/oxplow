//! A scripted **work-items provider** speaking oxplow's provider protocol
//! (P5.D2, `.context/providers.md`) — what the host, the instances and the
//! conformance kit are tested against, the way `oxplow-acp-fake` stands in
//! for an agent.
//!
//! It keeps items in memory (`work_item:fake:W-<n>`), with native states
//! `Backlog` / `Doing` / `Stuck` / `Shipped` / `Dropped` for the canonical
//! `todo` / `in_progress` / `blocked` / `done` / `canceled`. Its config
//! needs a `team` (a string); a clean `check` returns the handle
//! `fake:<team>`. Commands: `create`, `update`, `transition`, `link`,
//! `comment` — each returns `work_item.recorded` events (the item as it
//! now stands). Collector `work_items` streams every item after the
//! cursor as `$/record`, then `$/state { cursor }`.
//!
//! **Hooks**, from `OXPLOW_FAKE_HOOKS` at start or a `fake/hooks { hooks }`
//! notification later (comma-separated):
//! - `fail-next:<n>` — the next `n` `check` / `invoke` / `read` calls fail
//!   (`Internal`);
//! - `slow-check:<ms>` — every `check` takes `ms` first;
//! - `slow:<ms>` — every `invoke` and `read` takes `ms` first (and honours
//!   `$/cancel` meanwhile);
//! - `crash` — drop the connection on the next request (the binary exits
//!   with status 3);
//! - `bad-declarations` — `initialize` declares an extra command the
//!   checked-in declarations don't have.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::events::schema::{EventSchemaRegistry, EventType, WorkItemRecorded};
use oxplow_domain::work_items::{CanonicalState, WorkItemRecord};
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Id, Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

pub const PROVIDER: &str = "fake";

/// The script hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hooks {
    pub fail_next: u32,
    pub slow_ms: u64,
    pub slow_check_ms: u64,
    pub crash: bool,
    pub bad_declarations: bool,
}

impl Hooks {
    pub fn parse(spec: &str) -> Hooks {
        let mut hooks = Hooks::default();
        hooks.apply(spec);
        hooks
    }

    fn apply(&mut self, spec: &str) {
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once(':') {
                Some(("fail-next", n)) => self.fail_next = n.parse().unwrap_or(1),
                Some(("slow", ms)) => self.slow_ms = ms.parse().unwrap_or(0),
                Some(("slow-check", ms)) => self.slow_check_ms = ms.parse().unwrap_or(0),
                None if part == "fail-next" => self.fail_next = 1,
                None if part == "crash" => self.crash = true,
                None if part == "bad-declarations" => self.bad_declarations = true,
                _ => {}
            }
        }
    }
}

fn native_of(state: CanonicalState) -> &'static str {
    match state {
        CanonicalState::Todo => "Backlog",
        CanonicalState::InProgress => "Doing",
        CanonicalState::Blocked => "Stuck",
        CanonicalState::Done => "Shipped",
        CanonicalState::Canceled => "Dropped",
    }
}

fn canonical_of(native: &str) -> Option<CanonicalState> {
    CanonicalState::ALL
        .into_iter()
        .find(|s| native_of(*s).eq_ignore_ascii_case(native))
}

fn command(name: &str, summary: &str, input_schema: Value) -> CommandDecl {
    CommandDecl {
        name: name.into(),
        summary: summary.into(),
        input_schema,
        confirm: "never".into(),
        effect: "record".into(),
        undoable: false,
    }
}

/// What the fake declares — the checked-in declarations a host approves.
pub fn declarations() -> InitializeResult {
    let string = json!({ "type": "string" });
    let recorded_schema = EventSchemaRegistry::core()
        .schema(WorkItemRecorded::TYPE, WorkItemRecorded::V)
        .cloned()
        .unwrap_or(Value::Null);
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "work_items".into(),
            features: json!({
                "hierarchy": true, "comments": true, "links": true,
                "in_progress_opens_effort": false
            }),
        }],
        commands: vec![
            command(
                "create",
                "Create a work item.",
                json!({ "type": "object", "required": ["title"],
                        "properties": { "title": string, "body": string, "parent_ref": string } }),
            ),
            command(
                "update",
                "Edit a work item's title, body or parent.",
                json!({ "type": "object", "required": ["ref"],
                        "properties": { "ref": string, "title": string, "body": string, "parent_ref": string } }),
            ),
            command(
                "transition",
                "Move a work item to a canonical or native state.",
                json!({ "type": "object", "required": ["ref", "to"],
                        "properties": { "ref": string, "to": string } }),
            ),
            command(
                "link",
                "Link one work item to another.",
                json!({ "type": "object", "required": ["ref", "target", "link_type"],
                        "properties": { "ref": string, "target": string, "link_type": string } }),
            ),
            command(
                "comment",
                "Comment on a work item.",
                json!({ "type": "object", "required": ["ref", "body"],
                        "properties": { "ref": string, "body": string } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: WorkItemRecorded::TYPE.into(),
            v: WorkItemRecorded::V,
            schema: recorded_schema,
        }],
        collectors: vec![CollectorDecl {
            name: "work_items".into(),
            entity: "work_item".into(),
            description: "Every work item, after the cursor.".into(),
        }],
        config_schema: json!({ "type": "object", "required": ["team"],
                               "properties": { "team": { "type": "string" } } }),
    }
}

struct Item {
    record: WorkItemRecord,
    links: Vec<(String, String)>,
    comments: Vec<String>,
}

#[derive(Default)]
struct World {
    hooks: Hooks,
    items: BTreeMap<u64, Item>,
    next: u64,
    in_flight: HashMap<Id, oneshot::Sender<()>>,
}

type Shared = Arc<Mutex<World>>;

/// How serving ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    /// The stream ended, or `shutdown`.
    Ended,
    /// The `crash` hook fired: the connection is dropped without a reply
    /// (the binary exits non-zero).
    Crashed,
}

/// Serve the protocol on `reader` / `writer` until the stream ends, a
/// `shutdown`, or a `crash` hook.
pub async fn serve<R, W>(reader: R, writer: W, hooks: &str) -> Served
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (peer, mut incoming) = Peer::spawn(reader, writer);
    let world: Shared = Arc::new(Mutex::new(World {
        hooks: Hooks::parse(hooks),
        next: 1,
        ..World::default()
    }));
    while let Some(message) = incoming.recv().await {
        match message {
            Incoming::Notification { method, params } if method == notify::CANCEL => {
                if let Some(id) = params.get("id").and_then(Value::as_u64) {
                    if let Some(stop) = world.lock().await.in_flight.remove(&id) {
                        let _ = stop.send(());
                    }
                }
            }
            Incoming::Notification { method, params } if method == "fake/hooks" => {
                let spec = params.get("hooks").and_then(Value::as_str).unwrap_or("");
                world.lock().await.hooks.apply(spec);
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                if world.lock().await.hooks.crash {
                    return Served::Crashed;
                }
                if method == method::SHUTDOWN {
                    let _ = peer.respond(id, Ok(Value::Null)).await;
                    return Served::Ended;
                }
                let (stop_tx, stop_rx) = oneshot::channel();
                world.lock().await.in_flight.insert(id, stop_tx);
                let (peer, world) = (peer.clone(), world.clone());
                tokio::spawn(async move {
                    let result = tokio::select! {
                        r = handle(&peer, &world, id, &method, params) => r,
                        _ = stop_rx => Err(ProtocolError::Cancelled),
                    };
                    world.lock().await.in_flight.remove(&id);
                    let _ = peer.respond(id, result).await;
                });
            }
        }
    }
    Served::Ended
}

async fn take_failure(world: &Shared) -> Result<(), ProtocolError> {
    let mut w = world.lock().await;
    if w.hooks.fail_next > 0 {
        w.hooks.fail_next -= 1;
        return Err(ProtocolError::Internal(
            "scripted failure (fail-next)".into(),
        ));
    }
    Ok(())
}

async fn slow(world: &Shared) {
    let ms = world.lock().await.hooks.slow_ms;
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params).map_err(|e| ProtocolError::InvalidParams(e.to_string()))
}

fn handle_of(team: &str) -> Handle {
    Handle(format!("fake:{team}"))
}

fn require_handle(handle: &Handle) -> Result<(), ProtocolError> {
    if handle.0.starts_with("fake:") {
        Ok(())
    } else {
        Err(ProtocolError::NotConfigured(format!(
            "`{}` isn't a checked instance",
            handle.0
        )))
    }
}

async fn handle(
    peer: &Peer,
    world: &Shared,
    id: Id,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    match method {
        method::INITIALIZE => {
            let p: InitializeParams = parse(params)?;
            if p.protocol_version != PROTOCOL_VERSION {
                return Err(ProtocolError::InvalidParams(format!(
                    "protocol {} unsupported; this provider speaks {PROTOCOL_VERSION}",
                    p.protocol_version
                )));
            }
            let mut declared = declarations();
            if world.lock().await.hooks.bad_declarations {
                declared.commands.push(command(
                    "undeclared",
                    "A command the checked-in declarations don't list.",
                    json!({ "type": "object" }),
                ));
            }
            Ok(serde_json::to_value(declared).expect("declarations serialize"))
        }
        method::CHECK => {
            take_failure(world).await?;
            let ms = world.lock().await.hooks.slow_check_ms;
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            let p: CheckParams = parse(params)?;
            let team = p.config.get("team").and_then(Value::as_str);
            let result = match team {
                Some(team) if !team.is_empty() => CheckResult {
                    problems: Vec::new(),
                    handle: Some(handle_of(team)),
                },
                _ => CheckResult {
                    problems: vec![Problem {
                        path: "/team".into(),
                        message: "a team is required".into(),
                    }],
                    handle: None,
                },
            };
            Ok(serde_json::to_value(result).expect("check result serializes"))
        }
        method::DISCOVER => {
            let p: DiscoverParams = parse(params)?;
            require_handle(&p.handle)?;
            Ok(serde_json::to_value(DiscoverResult {
                entities: vec![EntityDecl {
                    name: "work_item".into(),
                    description: "A work item as the fake has it.".into(),
                    schema: json!({ "type": "object" }),
                }],
            })
            .expect("discover result serializes"))
        }
        method::INVOKE => {
            take_failure(world).await?;
            let p: InvokeParams = parse(params)?;
            require_handle(&p.handle)?;
            slow(world).await;
            invoke(world, &p.command, p.input).await
        }
        method::READ => {
            take_failure(world).await?;
            let p: ReadParams = parse(params)?;
            require_handle(&p.handle)?;
            if p.collector != "work_items" {
                return Err(ProtocolError::InvalidInput {
                    field: "/collector".into(),
                    message: format!("no collector `{}`", p.collector),
                });
            }
            slow(world).await;
            let after = p
                .state
                .as_ref()
                .and_then(|s| s.get("cursor"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let rows: Vec<(u64, Value)> = world
                .lock()
                .await
                .items
                .range(after + 1..)
                .map(|(n, item)| {
                    (
                        *n,
                        serde_json::to_value(&item.record).expect("record serializes"),
                    )
                })
                .collect();
            let mut cursor = after;
            for (n, row) in &rows {
                peer.notify(
                    notify::RECORD,
                    json!({ "id": id, "entity": "work_item", "row": row }),
                )
                .await?;
                cursor = *n;
            }
            peer.notify(
                notify::STATE,
                json!({ "id": id, "state": { "cursor": cursor } }),
            )
            .await?;
            Ok(json!({ "records": rows.len() }))
        }
        other => Err(ProtocolError::MethodNotFound(other.into())),
    }
}

fn number_of(item_ref: &str) -> Result<u64, ProtocolError> {
    item_ref
        .strip_prefix("work_item:fake:W-")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: "/ref".into(),
            message: format!("`{item_ref}` isn't a fake work item (work_item:fake:W-<n>)"),
        })
}

fn str_field(input: &Value, field: &str) -> Result<String, ProtocolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: format!("/{field}"),
            message: "required".into(),
        })
}

fn recorded(record: &WorkItemRecord) -> EventDraft {
    EventDraft {
        event_type: WorkItemRecorded::TYPE.into(),
        v: WorkItemRecorded::V,
        payload: json!({ "item": record }),
        subject: vec![record.item_ref.clone()],
    }
}

async fn invoke(world: &Shared, command: &str, input: Value) -> Result<Value, ProtocolError> {
    let mut w = world.lock().await;
    let (result, events) = match command {
        "create" => {
            let title = str_field(&input, "title")?;
            let parent_ref = input
                .get("parent_ref")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(parent) = &parent_ref {
                let n = number_of(parent)?;
                if !w.items.contains_key(&n) {
                    return Err(ProtocolError::InvalidInput {
                        field: "/parent_ref".into(),
                        message: format!("no item `{parent}`"),
                    });
                }
            }
            let n = w.next;
            w.next += 1;
            let record = WorkItemRecord {
                item_ref: format!("work_item:fake:W-{n}"),
                title,
                body: input
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                state: CanonicalState::Todo,
                native_state: native_of(CanonicalState::Todo).into(),
                native: json!({ "team": "fake" }),
                parent_ref,
                deleted: false,
            };
            let events = vec![recorded(&record)];
            let result = json!({ "ref": record.item_ref });
            w.items.insert(
                n,
                Item {
                    record,
                    links: Vec::new(),
                    comments: Vec::new(),
                },
            );
            (result, events)
        }
        "update" | "transition" | "link" | "comment" => {
            let item_ref = str_field(&input, "ref")?;
            let n = number_of(&item_ref)?;
            let item = w
                .items
                .get_mut(&n)
                .ok_or_else(|| ProtocolError::InvalidInput {
                    field: "/ref".into(),
                    message: format!("no item `{item_ref}`"),
                })?;
            match command {
                "update" => {
                    if let Some(t) = input.get("title").and_then(Value::as_str) {
                        item.record.title = t.into();
                    }
                    if let Some(b) = input.get("body").and_then(Value::as_str) {
                        item.record.body = b.into();
                    }
                    if let Some(p) = input.get("parent_ref").and_then(Value::as_str) {
                        item.record.parent_ref = (!p.is_empty()).then(|| p.to_string());
                    }
                }
                "transition" => {
                    let to = str_field(&input, "to")?;
                    let state = serde_json::from_value::<CanonicalState>(Value::String(to.clone()))
                        .ok()
                        .or_else(|| canonical_of(&to))
                        .ok_or_else(|| ProtocolError::InvalidInput {
                            field: "/to".into(),
                            message: format!("`{to}` is neither a canonical nor a fake state"),
                        })?;
                    item.record.state = state;
                    item.record.native_state = native_of(state).into();
                }
                "link" => {
                    item.links.push((
                        str_field(&input, "target")?,
                        str_field(&input, "link_type")?,
                    ));
                }
                _ => item.comments.push(str_field(&input, "body")?),
            }
            let events = vec![recorded(&item.record)];
            (json!({ "ref": item_ref }), events)
        }
        other => {
            return Err(ProtocolError::InvalidInput {
                field: "/command".into(),
                message: format!("no command `{other}`"),
            })
        }
    };
    Ok(serde_json::to_value(InvokeResult {
        result,
        events,
        inverse: None,
    })
    .expect("invoke result serializes"))
}
