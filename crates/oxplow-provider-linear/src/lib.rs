//! oxplow's reference **work-items provider**: Linear issues over Linear's
//! GraphQL API, speaking oxplow's provider protocol on stdio
//! (`.context/providers.md` → "The Linear provider").
//!
//! An instance is one team (`config.team`, its key — `ENG`), optionally
//! narrowed to one project (`config.project`, its name); `check` resolves
//! both and the team's workflow states and returns the handle
//! `linear:<team>[/<project>]`. An item is `work_item:linear:<identifier>`
//! (`ENG-12`), its uuid, url and priority under `native`. States map by
//! type, with the team state `config.blocked_state` (default "Blocked") as
//! blocked ([`states`]). The work-items verbs `create`, `update`,
//! `transition` (undoable), `link`, `comment` and `delete` run Linear's
//! `issueCreate` / `issueUpdate` / `issueRelationCreate` /
//! `commentCreate` / `issueDelete` and record the issue as it now stands.
//! Collector `issues` pages through the team's issues updated after its
//! cursor (`$/state { since, after, until }`), each page a checkpoint and a
//! `$/progress`. The API key is the credential `LINEAR_API_KEY` (the
//! process environment); `LINEAR_API_URL` overrides the endpoint.

pub mod graphql;
pub mod issue;
pub mod sim;
pub mod states;

use oxplow_domain::vocabulary::Vocabulary;
use std::collections::HashMap;
use std::sync::Arc;

use oxplow_domain::events::schema::{EventType, WorkItemRecorded};
use oxplow_domain::work_items::{CanonicalState, WorkItemRecord};
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Id, Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

use crate::graphql::Client;
use crate::issue::{identifier_of, ref_prefix};
use crate::states::{State, Team, DEFAULT_BLOCKED};

pub const PROVIDER: &str = "linear";
/// The credential holding the API key.
pub const API_KEY: &str = "LINEAR_API_KEY";
/// Issues asked for per page.
pub const PAGE: u64 = 50;

/// Where the process finds Linear, and which provider it is.
#[derive(Debug, Clone)]
pub struct Env {
    pub url: String,
    pub key: Option<String>,
    /// The provider id its manifest gives it — its refs' segment
    /// (`work_item:<id>:ENG-12`), so two entries are two instances.
    pub provider_id: String,
}

impl Env {
    /// From `LINEAR_API_URL` (default [`graphql::DEFAULT_URL`]),
    /// `LINEAR_API_KEY` and `OXPLOW_PROVIDER_ID` (default `linear`).
    pub fn from_process() -> Env {
        Env {
            url: std::env::var("LINEAR_API_URL").unwrap_or_else(|_| graphql::DEFAULT_URL.into()),
            key: std::env::var(API_KEY).ok().filter(|k| !k.is_empty()),
            provider_id: std::env::var("OXPLOW_PROVIDER_ID")
                .ok()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| PROVIDER.into()),
        }
    }
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

/// What it declares — the example extension's `provider.json`.
pub fn declarations() -> InitializeResult {
    let string = json!({ "type": "string" });
    let state = json!({ "type": "string",
                        "enum": ["todo", "in_progress", "blocked", "done", "canceled"] });
    let native = json!({ "type": "object", "additionalProperties": false,
                         "properties": { "priority": { "type": "integer", "minimum": 0, "maximum": 4 } } });
    let recorded_schema = Vocabulary::core()
        .schema(WorkItemRecorded::TYPE, WorkItemRecorded::V)
        .cloned()
        .unwrap_or(Value::Null);
    let mut transition = command(
        "transition",
        "Move an issue to a canonical state, optionally naming the team's workflow state.",
        json!({ "type": "object", "required": ["ref", "to"], "additionalProperties": false,
                "properties": { "ref": string, "to": state, "native_state": string } }),
    );
    transition.undoable = true;
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "work_items".into(),
            features: json!({
                "hierarchy": true, "comments": true, "links": true, "delete": true,
                "in_progress_opens_effort": false
            }),
        }],
        commands: vec![
            command(
                "create",
                "Create an issue in the instance's team (and project).",
                json!({ "type": "object", "required": ["title"], "additionalProperties": false,
                        "properties": { "title": string, "body": string, "parent_ref": string,
                                        "state": state, "native_state": string,
                                        "native": native } }),
            ),
            command(
                "update",
                "Edit an issue's title, description, parent, state or priority.",
                json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                        "properties": { "ref": string, "title": string, "body": string,
                                        "parent_ref": string, "state": state,
                                        "native_state": string, "native": native } }),
            ),
            transition,
            command(
                "link",
                "Relate one issue to another (blocks, relates_to or duplicates).",
                json!({ "type": "object", "required": ["ref", "target", "link_type"],
                        "additionalProperties": false,
                        "properties": { "ref": string, "target": string,
                                        "link_type": { "type": "string",
                                                       "enum": ["blocks", "relates_to", "duplicates"] } } }),
            ),
            command(
                "comment",
                "Comment on an issue.",
                json!({ "type": "object", "required": ["ref", "body"],
                        "additionalProperties": false,
                        "properties": { "ref": string, "body": string } }),
            ),
            command(
                "delete",
                "Move an issue to Linear's trash.",
                json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                        "properties": { "ref": string } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: WorkItemRecorded::TYPE.into(),
            v: WorkItemRecorded::V,
            schema: recorded_schema,
        }],
        collectors: vec![CollectorDecl {
            name: "issues".into(),
            entity: "work_item".into(),
            description:
                "The team's issues (the project's, when one is set) updated after the cursor."
                    .into(),
        }],
        config_schema: json!({
            "type": "object",
            "required": ["team"],
            "additionalProperties": false,
            "properties": {
                "team": { "type": "string", "description": "The team's key, as in its issue ids (ENG)." },
                "project": { "type": "string", "description": "Only this project's issues (its name)." },
                "blocked_state": { "type": "string", "default": DEFAULT_BLOCKED,
                                   "description": "The team's workflow state that means blocked." }
            }
        }),
    }
}

/// A checked instance: its refs' prefix, its team and project.
#[derive(Clone)]
struct Instance {
    prefix: String,
    team: Team,
    project: Option<String>,
}

impl Instance {
    fn record(&self, node: &Value) -> Result<WorkItemRecord, ProtocolError> {
        issue::record(&self.prefix, &self.team, node)
    }

    fn identifier<'a>(&self, item_ref: &'a str, field: &str) -> Result<&'a str, ProtocolError> {
        identifier_of(&self.prefix, item_ref, field)
    }
}

struct World {
    env: Env,
    instances: HashMap<String, Instance>,
    in_flight: HashMap<Id, oneshot::Sender<()>>,
}

type Shared = Arc<Mutex<World>>;

/// Serve the protocol on `reader` / `writer` until the stream ends or a
/// `shutdown`.
pub async fn serve<R, W>(reader: R, writer: W, env: Env)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (peer, mut incoming) = Peer::spawn(reader, writer);
    let world: Shared = Arc::new(Mutex::new(World {
        env,
        instances: HashMap::new(),
        in_flight: HashMap::new(),
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
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                if method == method::SHUTDOWN {
                    let _ = peer.respond(id, Ok(Value::Null)).await;
                    return;
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
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params).map_err(|e| ProtocolError::InvalidParams(e.to_string()))
}

fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).expect("protocol types serialize")
}

/// The client and instance `handle` names.
async fn instance(world: &Shared, handle: &Handle) -> Result<(Client, Instance), ProtocolError> {
    let w = world.lock().await;
    let instance = w.instances.get(&handle.0).cloned().ok_or_else(|| {
        ProtocolError::NotConfigured(format!("`{}` isn't a checked instance", handle.0))
    })?;
    let key = w.env.key.clone().ok_or_else(|| ProtocolError::Auth {
        message: format!("{API_KEY} isn't set"),
        credential: Some(API_KEY.into()),
    })?;
    Ok((Client::new(&w.env.url, &key), instance))
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
            Ok(to_value(declarations()))
        }
        method::CHECK => {
            let p: CheckParams = parse(params)?;
            let result = check(world, p).await?;
            Ok(to_value(result))
        }
        method::DISCOVER => {
            let p: DiscoverParams = parse(params)?;
            instance(world, &p.handle).await?;
            Ok(to_value(DiscoverResult {
                entities: vec![EntityDecl {
                    name: "work_item".into(),
                    description: "A Linear issue as a work item.".into(),
                    schema: json!({ "type": "object" }),
                }],
            }))
        }
        method::INVOKE => {
            let p: InvokeParams = parse(params)?;
            let (client, instance) = instance(world, &p.handle).await?;
            Ok(to_value(
                invoke(
                    &client,
                    &instance,
                    &p.command,
                    &p.input,
                    p.idempotency_key.as_deref(),
                )
                .await?,
            ))
        }
        method::READ => {
            let p: ReadParams = parse(params)?;
            let (client, instance) = instance(world, &p.handle).await?;
            if p.collector != "issues" {
                return Err(ProtocolError::InvalidInput {
                    field: "/collector".into(),
                    message: format!("no collector `{}`", p.collector),
                });
            }
            read(peer, id, &client, &instance, p.state.unwrap_or(Value::Null)).await
        }
        other => Err(ProtocolError::MethodNotFound(other.into())),
    }
}

fn problem(path: &str, message: impl Into<String>) -> CheckResult {
    CheckResult {
        problems: vec![Problem {
            path: path.into(),
            message: message.into(),
        }],
        handle: None,
    }
}

/// Resolve an instance's config against Linear: its team, its blocked
/// state and its project.
async fn check(world: &Shared, p: CheckParams) -> Result<CheckResult, ProtocolError> {
    let config = p.config.as_object().cloned().unwrap_or_default();
    if let Some(unknown) = config
        .keys()
        .find(|k| !["team", "project", "blocked_state"].contains(&k.as_str()))
    {
        return Ok(problem(&format!("/{unknown}"), "not a Linear setting"));
    }
    let Some(team_key) = config
        .get("team")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return Ok(problem("/team", "a team key is required (ENG)"));
    };
    let blocked = config
        .get("blocked_state")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_BLOCKED);
    let project = config.get("project").and_then(Value::as_str);
    let (url, key, provider_id) = {
        let w = world.lock().await;
        (
            w.env.url.clone(),
            w.env.key.clone(),
            w.env.provider_id.clone(),
        )
    };
    let Some(key) = key.filter(|_| p.credentials.iter().any(|c| c == API_KEY)) else {
        return Ok(problem(
            "",
            format!("the credential {API_KEY} isn't set: add a Linear API key"),
        ));
    };
    let client = Client::new(&url, &key);
    let teams = match client.run(issue::TEAM, json!({ "key": team_key })).await {
        Err(ProtocolError::Auth { message, .. }) => {
            return Ok(problem(
                "",
                format!("Linear refused the API key: {message}"),
            ))
        }
        other => other?,
    };
    let Some(node) = teams["teams"]["nodes"].get(0) else {
        return Ok(problem("/team", format!("no team with key `{team_key}`")));
    };
    let team = Team {
        id: node["id"].as_str().unwrap_or_default().to_string(),
        key: team_key.to_string(),
        states: node["states"]["nodes"]
            .as_array()
            .map(|s| s.iter().filter_map(State::from_node).collect())
            .unwrap_or_default(),
        blocked: blocked.to_string(),
    };
    if !team
        .states
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(blocked))
    {
        return Ok(problem(
            "/blocked_state",
            format!("team {team_key} has no `{blocked}` state: add one, or name the state that means blocked"),
        ));
    }
    let project_id = match project {
        None => None,
        Some(name) => {
            let found = client
                .run(issue::PROJECT, json!({ "team": team.id, "name": name }))
                .await?;
            match found["projects"]["nodes"][0]["id"].as_str() {
                Some(id) => Some(id.to_string()),
                None => {
                    return Ok(problem(
                        "/project",
                        format!("team {team_key} has no project `{name}`"),
                    ))
                }
            }
        }
    };
    let handle = match project {
        None => format!("linear:{team_key}"),
        Some(name) => format!("linear:{team_key}/{name}"),
    };
    world.lock().await.instances.insert(
        handle.clone(),
        Instance {
            prefix: ref_prefix(&provider_id),
            team,
            project: project_id,
        },
    );
    Ok(CheckResult {
        problems: Vec::new(),
        handle: Some(Handle(handle)),
    })
}

/// An `InvalidInput` from Linear (a missing issue) placed at `field`.
fn at(field: &'static str) -> impl Fn(ProtocolError) -> ProtocolError {
    move |e| match e {
        ProtocolError::InvalidInput { field: f, message } if f.is_empty() => {
            ProtocolError::InvalidInput {
                field: field.into(),
                message,
            }
        }
        other => other,
    }
}

fn required<'a>(input: &'a Value, field: &str) -> Result<&'a str, ProtocolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: format!("/{field}"),
            message: "required".into(),
        })
}

/// The workflow state `input` asks for through `field` (a canonical
/// state) and `native_state` (the team's state, which must be one of it).
fn target_state<'a>(
    team: &'a Team,
    input: &Value,
    field: &str,
) -> Result<Option<&'a State>, ProtocolError> {
    let to = match input.get(field).and_then(Value::as_str) {
        None => None,
        Some(raw) => Some(
            serde_json::from_value::<CanonicalState>(Value::String(raw.into())).map_err(|_| {
                ProtocolError::InvalidInput {
                    field: format!("/{field}"),
                    message: format!("`{raw}` isn't a canonical state"),
                }
            })?,
        ),
    };
    let native = input.get("native_state").and_then(Value::as_str);
    match (to, native) {
        (Some(to), native) => team.state_for(to, native).map(Some),
        (None, Some(name)) => team
            .states
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(Some)
            .ok_or_else(|| ProtocolError::InvalidInput {
                field: "/native_state".into(),
                message: format!("team {} has no state `{name}`", team.key),
            }),
        (None, None) => Ok(None),
    }
}

/// The `IssueCreateInput` / `IssueUpdateInput` fields the contract's
/// `body`, `parent_ref` (`""` detaches), state and `native.priority` set.
fn issue_input(
    instance: &Instance,
    input: &Value,
    state_field: &str,
    parent: Option<Value>,
) -> Result<serde_json::Map<String, Value>, ProtocolError> {
    let team = &instance.team;
    let mut out = serde_json::Map::new();
    if let Some(title) = input.get("title").and_then(Value::as_str) {
        out.insert("title".into(), json!(title));
    }
    if let Some(body) = input.get("body").and_then(Value::as_str) {
        out.insert("description".into(), json!(body));
    }
    if let Some(parent) = parent {
        out.insert("parentId".into(), parent);
    }
    if let Some(state) = target_state(team, input, state_field)? {
        out.insert("stateId".into(), json!(state.id));
    }
    if let Some(priority) = input.get("native").and_then(|n| n.get("priority")) {
        out.insert("priority".into(), priority.clone());
    }
    Ok(out)
}

fn recorded(record: &WorkItemRecord) -> EventDraft {
    EventDraft {
        event_type: WorkItemRecorded::TYPE.into(),
        v: WorkItemRecorded::V,
        payload: json!({ "item": record }),
        subject: vec![record.item_ref.clone()],
    }
}

fn written(record: WorkItemRecord, inverse: Option<CommandCall>) -> InvokeResult {
    InvokeResult {
        result: json!({ "ref": record.item_ref }),
        events: vec![recorded(&record)],
        inverse,
    }
}

/// The uuid of the issue `item_ref` names: what an input object's issue
/// field takes (`parentId`, `issueId`, `relatedIssueId` — only a top-level
/// `id` argument accepts the identifier). A missing issue is the input's
/// fault, at `field`.
async fn uuid_of(
    client: &Client,
    instance: &Instance,
    item_ref: &str,
    field: &'static str,
) -> Result<String, ProtocolError> {
    let id = instance.identifier(item_ref, field)?;
    let data = client
        .run(issue::ISSUE, json!({ "id": id }))
        .await
        .map_err(at(field))?;
    data["issue"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: field.into(),
            message: format!("no issue `{item_ref}`"),
        })
}

/// The `parentId` an input's `parent_ref` asks for: its uuid, `null` for
/// `""` (detach), `None` when not given.
async fn parent_of(
    client: &Client,
    instance: &Instance,
    input: &Value,
) -> Result<Option<Value>, ProtocolError> {
    match input.get("parent_ref").and_then(Value::as_str) {
        None => Ok(None),
        Some("") => Ok(Some(Value::Null)),
        Some(p) => Ok(Some(json!(
            uuid_of(client, instance, p, "/parent_ref").await?
        ))),
    }
}

/// The issue `id` names, as it stands.
async fn current(
    client: &Client,
    instance: &Instance,
    id: &str,
) -> Result<WorkItemRecord, ProtocolError> {
    let data = client
        .run(issue::ISSUE, json!({ "id": id }))
        .await
        .map_err(at("/ref"))?;
    instance.record(&data["issue"])
}

/// The id a create sends, derived from its idempotency key: the same
/// write sent again carries the same id, which Linear refuses as taken.
/// None without a key. Shaped as a v4 UUID — the form a client-chosen
/// Linear id takes — from the bytes of a v5 UUID over the verb and the
/// key (tsk931), so its shape is never the question.
fn client_id(verb: &str, key: Option<&str>) -> Option<String> {
    key.map(|key| {
        let derived = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("oxplow:linear:{verb}:{key}").as_bytes(),
        );
        uuid::Builder::from_random_bytes(*derived.as_bytes())
            .into_uuid()
            .to_string()
    })
}

/// Run create `op`, sending `id` (a client id) when there is one. A create
/// refused whose id already names what it would make was sent before and
/// landed: `lookup` reads that, and it is the answer. A rate limit or a
/// refused key never landed, and is the error as it is. When the id names
/// nothing, the refusal was the create's own; when the lookup itself
/// fails (rate limited, unreachable), that is the answer — the write may
/// have landed, and only the lookup can say (tsk931).
async fn create(
    client: &Client,
    op: graphql::Operation,
    mut input: serde_json::Map<String, Value>,
    id: Option<String>,
    lookup: graphql::Operation,
) -> Result<Value, ProtocolError> {
    if let Some(id) = &id {
        input.insert("id".into(), json!(id));
    }
    match client.run(op, json!({ "input": input })).await {
        Ok(data) => Ok(data),
        Err(e @ (ProtocolError::RateLimited { .. } | ProtocolError::Auth { .. })) => Err(e),
        Err(e) => match id {
            None => Err(e),
            Some(id) => match client.run(lookup, json!({ "id": id })).await {
                Ok(found) => Ok(found),
                Err(ProtocolError::InvalidInput { .. }) => Err(e),
                Err(lookup_failed) => Err(lookup_failed),
            },
        },
    }
}

async fn invoke(
    client: &Client,
    instance: &Instance,
    command: &str,
    input: &Value,
    key: Option<&str>,
) -> Result<InvokeResult, ProtocolError> {
    let team = &instance.team;
    let id = client_id(command, key);
    match command {
        "create" => {
            required(input, "title")?;
            let parent = parent_of(client, instance, input).await?;
            let mut fields = issue_input(instance, input, "state", parent)?;
            fields.insert("teamId".into(), json!(team.id));
            if let Some(project) = &instance.project {
                fields.insert("projectId".into(), json!(project));
            }
            // A refusal Linear attributes to no field stays field-less: the
            // parent was resolved before the create was sent (tsk946).
            let data = create(client, issue::ISSUE_CREATE, fields, id, issue::ISSUE).await?;
            // A fresh create answers `issueCreate.issue`; a repeated one,
            // looked up, `issue`.
            let issue = data["issueCreate"]["issue"]
                .as_object()
                .map_or(&data["issue"], |_| &data["issueCreate"]["issue"]);
            Ok(written(instance.record(issue)?, None))
        }
        "update" => {
            let id = instance.identifier(required(input, "ref")?, "/ref")?;
            let parent = parent_of(client, instance, input).await?;
            let fields = issue_input(instance, input, "state", parent)?;
            // A refusal Linear attributes to no field stays field-less: any
            // field sent may be what it refused, not only the ref (tsk1006).
            let data = client
                .run(issue::ISSUE_UPDATE, json!({ "id": id, "input": fields }))
                .await?;
            Ok(written(
                instance.record(&data["issueUpdate"]["issue"])?,
                None,
            ))
        }
        "transition" => {
            let item_ref = required(input, "ref")?;
            let id = instance.identifier(item_ref, "/ref")?;
            required(input, "to")?;
            let state =
                target_state(team, input, "to")?.ok_or_else(|| ProtocolError::InvalidInput {
                    field: "/to".into(),
                    message: "required".into(),
                })?;
            let before = current(client, instance, id).await?;
            let data = client
                .run(
                    issue::ISSUE_UPDATE,
                    json!({ "id": id, "input": { "stateId": state.id } }),
                )
                // Its ref was read just now: a refusal is of the move.
                .await?;
            let inverse = CommandCall {
                command: "transition".into(),
                input: json!({ "ref": item_ref, "to": before.state.as_str(),
                               "native_state": before.native_state }),
            };
            Ok(written(
                instance.record(&data["issueUpdate"]["issue"])?,
                Some(inverse),
            ))
        }
        "link" => {
            let item_ref = required(input, "ref")?;
            let target_ref = required(input, "target")?;
            instance.identifier(item_ref, "/ref")?;
            instance.identifier(target_ref, "/target")?;
            let kind = match required(input, "link_type")? {
                "blocks" => "blocks",
                "relates_to" => "related",
                "duplicates" => "duplicate",
                other => {
                    return Err(ProtocolError::InvalidInput {
                        field: "/link_type".into(),
                        message: format!(
                            "Linear has no `{other}` relation (blocks, relates_to, duplicates)"
                        ),
                    })
                }
            };
            let issue_id = uuid_of(client, instance, item_ref, "/ref").await?;
            let target = uuid_of(client, instance, target_ref, "/target").await?;
            let fields = json!({ "issueId": issue_id, "relatedIssueId": target, "type": kind });
            let data = create(
                client,
                issue::RELATION_CREATE,
                fields.as_object().cloned().unwrap_or_default(),
                id,
                issue::RELATION,
            )
            // Both refs were resolved first: a refusal is of the link.
            .await?;
            let relation = data["issueRelationCreate"]["issueRelation"]
                .as_object()
                .map_or(&data["issueRelation"], |_| {
                    &data["issueRelationCreate"]["issueRelation"]
                });
            Ok(written(instance.record(&relation["issue"])?, None))
        }
        "comment" => {
            let body = required(input, "body")?;
            let issue_id = uuid_of(client, instance, required(input, "ref")?, "/ref").await?;
            let fields = json!({ "issueId": issue_id, "body": body });
            let data = create(
                client,
                issue::COMMENT_CREATE,
                fields.as_object().cloned().unwrap_or_default(),
                id,
                issue::COMMENT,
            )
            // Its ref was resolved first: a refusal is of the comment.
            .await?;
            let comment = data["commentCreate"]["comment"]
                .as_object()
                .map_or(&data["comment"], |_| &data["commentCreate"]["comment"]);
            Ok(written(instance.record(&comment["issue"])?, None))
        }
        "delete" => {
            let id = instance.identifier(required(input, "ref")?, "/ref")?;
            let mut gone = current(client, instance, id).await?;
            client
                .run(issue::ISSUE_DELETE, json!({ "id": id }))
                .await
                .map_err(at("/ref"))?;
            gone.deleted = true;
            Ok(written(gone, None))
        }
        other => Err(ProtocolError::InvalidInput {
            field: "/command".into(),
            message: format!("no command `{other}`"),
        }),
    }
}

/// Collector `issues`: every issue of the instance's team (and project)
/// updated after `state.since`, a page at a time from `state.after`. Each
/// page streams its records, then checkpoints `{ since, after, until }`
/// (`until`: the latest update seen); the last one `{ since: until }`, so
/// the next read starts after everything this one saw.
async fn read(
    peer: &Peer,
    id: Id,
    client: &Client,
    instance: &Instance,
    state: Value,
) -> Result<Value, ProtocolError> {
    let since = state["since"].as_str().map(str::to_string);
    let mut after = state["after"].as_str().map(str::to_string);
    let mut until = state["until"]
        .as_str()
        .map(str::to_string)
        .or(since.clone());
    let mut filter = json!({ "team": { "id": { "eq": instance.team.id } } });
    if let Some(since) = &since {
        filter["updatedAt"] = json!({ "gt": since });
    }
    if let Some(project) = &instance.project {
        filter["project"] = json!({ "id": { "eq": project } });
    }
    let mut total = 0u64;
    let mut page = 0u64;
    loop {
        page += 1;
        peer.notify(
            notify::PROGRESS,
            json!({ "id": id, "message": format!("issues: page {page}") }),
        )
        .await?;
        let data = client
            .run(
                issue::ISSUES,
                json!({ "filter": filter, "first": PAGE, "after": after }),
            )
            .await?;
        let issues = &data["issues"];
        for node in issues["nodes"].as_array().into_iter().flatten() {
            if let Some(updated) = node["updatedAt"].as_str() {
                if until.as_deref().is_none_or(|u| updated > u) {
                    until = Some(updated.to_string());
                }
            }
            // An issue the provider can't map is skipped and said, never
            // a failed read: the checkpoint moves past it, so one odd
            // issue can't fail every read until the instance disables.
            let row = match instance.record(node) {
                Ok(row) => row,
                Err(e) => {
                    let message = format!("issues: skipped an issue: {e}");
                    eprintln!("oxplow-provider-linear: {message}");
                    peer.notify(notify::PROGRESS, json!({ "id": id, "message": message }))
                        .await?;
                    continue;
                }
            };
            peer.notify(
                notify::RECORD,
                json!({ "id": id, "entity": "work_item", "row": row }),
            )
            .await?;
            total += 1;
        }
        let more = issues["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
        after = issues["pageInfo"]["endCursor"].as_str().map(str::to_string);
        let checkpoint = if more {
            json!({ "since": since, "after": after, "until": until })
        } else {
            json!({ "since": until })
        };
        peer.notify(notify::STATE, json!({ "id": id, "state": checkpoint }))
            .await?;
        if !more {
            return Ok(json!({ "records": total }));
        }
    }
}

#[cfg(test)]
mod tests {
    /// tsk931: a create's client id is a v4-shaped UUID, the same for the
    /// same verb and key.
    #[test]
    fn a_client_id_is_v4_shaped_and_stable() {
        let id = super::client_id("create", Some("k")).unwrap();
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(super::client_id("create", Some("k")).unwrap(), id);
        assert_ne!(super::client_id("comment", Some("k")).unwrap(), id);
        assert_eq!(super::client_id("create", None), None);
    }
}
