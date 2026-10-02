//! oxplow's **MCP adapter** (P7.A6, `.context/providers.md` → "The MCP
//! adapter"): a provider made of an MCP server and a mapping. The host
//! runs this program (oxplow's own, shipped beside it) in the extension's
//! folder, with what its manifest's `adapter:` names:
//!
//! ```text
//! oxplow-provider-mcp --declarations provider.json --mapping mcp/x.star \
//!     --tools mcp/tools.json -- bin/server --stdio
//! ```
//!
//! It answers `initialize` with the checked-in declarations, and starts
//! the server (an MCP client over its stdio) at `check`, refusing it when
//! its tools — each one's name, description and input schema — aren't
//! exactly the pinned `tools.json`. `invoke` and `read` run the mapping, a
//! Starlark `transform(x)` under the sandbox (5 s), twice: once to turn
//! the request into a tool call (`x.phase` `invoke` / `read`), once to
//! turn the tool's output into the answer (`invoked` / `records`). Tool
//! output is data — the mapping reads it, nothing runs it — and what the
//! mapping returns is checked: only declared event types, only this
//! provider's refs (`work_item:<id>:…`, the id from
//! `OXPLOW_PROVIDER_ID`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_collect_plugin::runtime::{run_sandboxed, run_starlark};
use oxplow_collect_plugin::SandboxBudget;
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Id, Incoming, Peer, ProtocolError};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

/// How long one run of the mapping may take.
pub const MAPPING_TIMEOUT: Duration = Duration::from_secs(5);

/// What the adapter runs: from its arguments, the files read.
#[derive(Debug, Clone)]
pub struct Adapter {
    pub declarations: InitializeResult,
    /// The mapping's Starlark source.
    pub mapping: String,
    /// The pinned tools (`tools.json`): `[{ name, description, inputSchema }]`.
    pub tools: Vec<Value>,
    /// The MCP server's command, its program in the extension folder.
    pub server: Vec<String>,
    /// Its provider id: its refs are `work_item:<id>:…`.
    pub provider_id: String,
}

impl Adapter {
    /// From `--declarations <file> --mapping <file> --tools <file> --
    /// <server command…>`, files relative to `dir`.
    pub fn from_args(dir: &Path, args: &[String], provider_id: &str) -> Result<Adapter, String> {
        let split = args
            .iter()
            .position(|a| a == "--")
            .ok_or("no `-- <server command>`")?;
        let (flags, server) = (&args[..split], &args[split + 1..]);
        if server.is_empty() {
            return Err("no server command after `--`".into());
        }
        let flag = |name: &str| -> Result<String, String> {
            let at = flags
                .iter()
                .position(|f| f == name)
                .ok_or_else(|| format!("missing {name}"))?;
            let path = flags
                .get(at + 1)
                .ok_or_else(|| format!("{name} needs a file"))?;
            std::fs::read_to_string(dir.join(path)).map_err(|e| format!("{name} {path}: {e}"))
        };
        let declarations = serde_json::from_str(&flag("--declarations")?)
            .map_err(|e| format!("--declarations: {e}"))?;
        let tools: Value =
            serde_json::from_str(&flag("--tools")?).map_err(|e| format!("--tools: {e}"))?;
        let tools = tools
            .as_array()
            .cloned()
            .ok_or("--tools must be a JSON list of tools")?;
        // The server is the folder's file its approval covers, never a
        // name looked up on PATH.
        let mut server = server.to_vec();
        server[0] = dir.join(&server[0]).to_string_lossy().into_owned();
        Ok(Adapter {
            declarations,
            mapping: flag("--mapping")?,
            tools,
            server,
            provider_id: provider_id.to_string(),
        })
    }

    fn prefix(&self) -> String {
        format!("work_item:{}:", self.provider_id)
    }
}

/// A tool as `tools.json` pins it.
pub fn pinned(tool: &rmcp::model::Tool) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": Value::Object((*tool.input_schema).clone()),
    })
}

/// The first way `live` (the server's tools) differs from `pins`, or `None`.
pub fn pin_difference(pins: &[Value], live: &[Value]) -> Option<String> {
    let names = |tools: &[Value]| {
        let mut n: Vec<String> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap_or_default().to_string())
            .collect();
        n.sort();
        n
    };
    let (want, got) = (names(pins), names(live));
    if let Some(extra) = got.iter().find(|n| !want.contains(n)) {
        return Some(format!(
            "the server has a tool `{extra}` tools.json doesn't pin"
        ));
    }
    if let Some(gone) = want.iter().find(|n| !got.contains(n)) {
        return Some(format!(
            "the server has no tool `{gone}`, which tools.json pins"
        ));
    }
    for pin in pins {
        let name = pin["name"].as_str().unwrap_or_default();
        let Some(tool) = live.iter().find(|t| t["name"] == name) else {
            continue;
        };
        for field in ["description", "inputSchema"] {
            if pin.get(field).unwrap_or(&Value::Null) != tool.get(field).unwrap_or(&Value::Null) {
                return Some(format!(
                    "the server's `{name}` {field} isn't the pinned one"
                ));
            }
        }
    }
    None
}

type Client = RunningService<RoleClient, ()>;

struct World {
    adapter: Adapter,
    client: Option<Arc<Client>>,
    /// Checked instances: handle → config.
    configs: HashMap<String, Value>,
    in_flight: HashMap<Id, oneshot::Sender<()>>,
}

type Shared = Arc<Mutex<World>>;

/// Serve the protocol on `reader` / `writer` until the stream ends or a
/// `shutdown`; the server stops with it.
pub async fn serve<R, W>(reader: R, writer: W, adapter: Adapter)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (peer, mut incoming) = Peer::spawn(reader, writer);
    let world: Shared = Arc::new(Mutex::new(World {
        adapter,
        client: None,
        configs: HashMap::new(),
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
                    break;
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
    let client = world.lock().await.client.take();
    if let Some(client) = client.and_then(|c| Arc::try_unwrap(c).ok()) {
        let _ = client.cancel().await;
    }
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params).map_err(|e| ProtocolError::InvalidParams(e.to_string()))
}

fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).expect("protocol types serialize")
}

fn internal(message: impl Into<String>) -> ProtocolError {
    ProtocolError::Internal(message.into())
}

/// The running server, started (and its tools checked against the pin)
/// the first time.
async fn client(world: &Shared) -> Result<Result<Arc<Client>, String>, ProtocolError> {
    let mut w = world.lock().await;
    if let Some(c) = &w.client {
        return Ok(Ok(c.clone()));
    }
    let (program, args) = w
        .adapter
        .server
        .split_first()
        .ok_or_else(|| internal("no server"))?;
    let cmd = tokio::process::Command::new(program).configure(|c| {
        c.args(args);
    });
    let transport = TokioChildProcess::new(cmd)
        .map_err(|e| internal(format!("couldn't start the MCP server `{program}`: {e}")))?;
    let client = ()
        .serve(transport)
        .await
        .map_err(|e| internal(format!("the MCP server `{program}` didn't initialize: {e}")))?;
    let live: Vec<Value> = client
        .list_all_tools()
        .await
        .map_err(|e| internal(format!("tools/list failed: {e}")))?
        .iter()
        .map(pinned)
        .collect();
    if let Some(difference) = pin_difference(&w.adapter.tools, &live) {
        let _ = client.cancel().await;
        return Ok(Err(format!(
            "{difference}: a changed server needs its tools.json updated, and a person's approval"
        )));
    }
    let client = Arc::new(client);
    w.client = Some(client.clone());
    Ok(Ok(client))
}

/// Run the mapping's `transform(x)` under the sandbox budget.
async fn map(mapping: &str, x: Value) -> Result<Value, ProtocolError> {
    let script = mapping.to_string();
    let phase = x["phase"].as_str().unwrap_or_default().to_string();
    tokio::task::spawn_blocking(move || {
        let budget = SandboxBudget {
            timeout: MAPPING_TIMEOUT,
            ceiling: MAPPING_TIMEOUT,
        };
        run_sandboxed(&budget, move || run_starlark(&script, &x))
    })
    .await
    .map_err(|e| internal(format!("mapping: {e}")))?
    .map_err(|e| internal(format!("mapping (phase {phase}): {e}")))
}

/// A tool's output as the mapping sees it: its structured content, else
/// its text (as JSON when it parses).
fn output_of(result: &CallToolResult) -> Value {
    if let Some(structured) = &result.structured_content {
        return structured.clone();
    }
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

/// Call the tool a mapping's answer names (`{ tool, arguments }`), or the
/// refusal it returned instead (`{ refuse: { field, message } }`).
async fn call(client: &Client, call: &Value) -> Result<Value, ProtocolError> {
    if let Some(refuse) = call.get("refuse") {
        return Err(ProtocolError::InvalidInput {
            field: refuse["field"].as_str().unwrap_or_default().to_string(),
            message: refuse["message"].as_str().unwrap_or("refused").to_string(),
        });
    }
    let tool = call["tool"]
        .as_str()
        .ok_or_else(|| internal("the mapping named no `tool`"))?
        .to_string();
    let arguments = call["arguments"].as_object().cloned().unwrap_or_default();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.clone()).with_arguments(arguments))
        .await
        .map_err(|e| internal(format!("tool `{tool}`: {e}")))?;
    let output = output_of(&result);
    if result.is_error == Some(true) {
        return Err(internal(format!("tool `{tool}` failed: {output}")));
    }
    Ok(output)
}

/// Refuse what isn't this provider's: a ref outside its prefix.
fn own_ref(prefix: &str, r: &Value, what: &str) -> Result<(), ProtocolError> {
    match r.as_str() {
        Some(r) if r.starts_with(prefix) => Ok(()),
        _ => Err(internal(format!(
            "the mapping returned {what} `{r}`, which isn't one of this provider's ({prefix}…)"
        ))),
    }
}

/// The mapping's `invoked` answer as an `InvokeResult`, checked: declared
/// event types only, its own refs only.
fn invoke_result(adapter: &Adapter, answer: Value) -> Result<InvokeResult, ProtocolError> {
    let result: InvokeResult = serde_json::from_value(json!({
        "result": answer.get("result").cloned().unwrap_or(Value::Null),
        "events": answer.get("events").cloned().unwrap_or(json!([])),
        "inverse": answer.get("inverse").cloned(),
    }))
    .map_err(|e| internal(format!("the mapping's answer: {e}")))?;
    let prefix = adapter.prefix();
    for e in &result.events {
        if !adapter
            .declarations
            .event_types
            .iter()
            .any(|d| d.event_type == e.event_type && d.v == e.v)
        {
            return Err(internal(format!(
                "the mapping returned a `{}@{}` event, which the provider doesn't declare",
                e.event_type, e.v
            )));
        }
        for s in &e.subject {
            own_ref(&prefix, &json!(s), "a subject")?;
        }
        if let Some(item) = e.payload.get("item") {
            own_ref(&prefix, &item["ref"], "an item")?;
        }
    }
    Ok(result)
}

async fn handle(
    peer: &Peer,
    world: &Shared,
    id: Id,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    let adapter = world.lock().await.adapter.clone();
    match method {
        method::INITIALIZE => {
            let p: InitializeParams = parse(params)?;
            if p.protocol_version != PROTOCOL_VERSION {
                return Err(ProtocolError::InvalidParams(format!(
                    "protocol {} unsupported; this adapter speaks {PROTOCOL_VERSION}",
                    p.protocol_version
                )));
            }
            Ok(to_value(&adapter.declarations))
        }
        method::CHECK => {
            let p: CheckParams = parse(params)?;
            let result = match client(world).await? {
                Err(pin) => CheckResult {
                    problems: vec![Problem {
                        path: String::new(),
                        message: pin,
                    }],
                    handle: None,
                },
                Ok(_) => {
                    let mut w = world.lock().await;
                    let handle = format!("{}:{}", adapter.provider_id, w.configs.len() + 1);
                    w.configs.insert(handle.clone(), p.config);
                    CheckResult {
                        problems: Vec::new(),
                        handle: Some(Handle(handle)),
                    }
                }
            };
            Ok(to_value(result))
        }
        method::DISCOVER => {
            let p: DiscoverParams = parse(params)?;
            config(world, &p.handle).await?;
            let entities = adapter
                .declarations
                .collectors
                .iter()
                .map(|c| EntityDecl {
                    name: c.entity.clone(),
                    description: c.description.clone(),
                    schema: json!({ "type": "object" }),
                })
                .collect();
            Ok(to_value(DiscoverResult { entities }))
        }
        method::INVOKE => {
            let p: InvokeParams = parse(params)?;
            let config = config(world, &p.handle).await?;
            let client = running(world).await?;
            let mut x = json!({ "phase": "invoke", "command": p.command, "input": p.input,
                                "config": config, "provider": adapter.provider_id });
            let tool_call = map(&adapter.mapping, x.clone()).await?;
            x["phase"] = json!("invoked");
            x["output"] = call(&client, &tool_call).await?;
            let answer = map(&adapter.mapping, x).await?;
            Ok(to_value(invoke_result(&adapter, answer)?))
        }
        method::READ => {
            let p: ReadParams = parse(params)?;
            let config = config(world, &p.handle).await?;
            let collector = adapter
                .declarations
                .collectors
                .iter()
                .find(|c| c.name == p.collector)
                .cloned()
                .ok_or_else(|| ProtocolError::InvalidInput {
                    field: "/collector".into(),
                    message: format!("no collector `{}`", p.collector),
                })?;
            let client = running(world).await?;
            let mut x = json!({ "phase": "read", "collector": p.collector, "state": p.state,
                                "config": config, "provider": adapter.provider_id });
            let tool_call = map(&adapter.mapping, x.clone()).await?;
            x["phase"] = json!("records");
            x["output"] = call(&client, &tool_call).await?;
            let answer = map(&adapter.mapping, x).await?;
            let prefix = adapter.prefix();
            let rows = answer["records"].as_array().cloned().unwrap_or_default();
            for row in &rows {
                own_ref(&prefix, &row["ref"], "a record")?;
            }
            for row in &rows {
                peer.notify(
                    notify::RECORD,
                    json!({ "id": id, "entity": collector.entity, "row": row }),
                )
                .await?;
            }
            peer.notify(notify::STATE, json!({ "id": id, "state": answer["state"] }))
                .await?;
            Ok(json!({ "records": rows.len() }))
        }
        other => Err(ProtocolError::MethodNotFound(other.into())),
    }
}

async fn config(world: &Shared, handle: &Handle) -> Result<Value, ProtocolError> {
    world
        .lock()
        .await
        .configs
        .get(&handle.0)
        .cloned()
        .ok_or_else(|| {
            ProtocolError::NotConfigured(format!("`{}` isn't a checked instance", handle.0))
        })
}

/// The server, which a clean `check` started.
async fn running(world: &Shared) -> Result<Arc<Client>, ProtocolError> {
    match client(world).await? {
        Ok(c) => Ok(c),
        Err(pin) => Err(ProtocolError::NotConfigured(pin)),
    }
}
