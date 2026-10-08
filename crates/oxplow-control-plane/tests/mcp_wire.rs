//! Wire-level test for the MCP surface: a real Streamable HTTP session
//! against `/mcp`, the way Claude Code talks to it. The oxplow-mcp unit
//! tests call tool methods directly and never cross rmcp's protocol layer,
//! which is the part each rmcp major reshapes (rmcp 3: `ServerConfig`,
//! `CallToolResponse`, list-result cache hints).

#![allow(clippy::unwrap_used)]

mod common;

use common::boot;
use serde_json::{json, Value};

/// POST one JSON-RPC message; return the response's session id (if any)
/// and, for requests, the JSON-RPC reply — rmcp answers either as plain
/// JSON or as an SSE stream whose `data:` line carries the message.
async fn post(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    session: Option<&str>,
    body: Value,
) -> (Option<String>, Option<Value>) {
    let mut req = client
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/json, text/event-stream")
        .json(&body);
    if let Some(s) = session {
        req = req.header("Mcp-Session-Id", s);
    }
    let resp = req.send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "HTTP {} for {body}",
        resp.status()
    );
    let session = resp
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let text = resp.text().await.unwrap();
    let reply = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .chain(std::iter::once(text.trim()))
        .filter_map(|chunk| serde_json::from_str::<Value>(chunk).ok())
        .find(|v| v.get("id").is_some());
    (session, reply)
}

#[tokio::test]
async fn mcp_session_initializes_lists_and_calls_tools_over_http() {
    let (cp, services, _root, _dir) = boot().await;
    let url = cp.mcp_endpoint_url();
    let token = common::bearer(&services, writer_thread(&services).await.id).await;
    let client = reqwest::Client::new();

    let (session, init) = post(
        &client,
        &url,
        &token,
        None,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "mcp-wire-test", "version": "0"}
            }
        }),
    )
    .await;
    let init = init.unwrap();
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    // The server's own instructions arrive over the wire.
    assert!(init["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("show_lens"));
    let session = session.expect("server assigns an Mcp-Session-Id");

    post(
        &client,
        &url,
        &token,
        Some(&session),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;

    let (_, list) = post(
        &client,
        &url,
        &token,
        Some(&session),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let list = list.unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from tools/list"))
    };
    // Our hand-rolled list_tools stamps read-only hints (tsk203) …
    assert_eq!(tool("ping")["annotations"]["readOnlyHint"], true);
    // … and leaves mutating tools gated.
    assert_ne!(tool("run_command")["annotations"]["readOnlyHint"], true);

    let (_, call) = post(
        &client,
        &url,
        &token,
        Some(&session),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "ping", "arguments": {}}
        }),
    )
    .await;
    let call = call.unwrap();
    assert_eq!(call["result"]["content"][0]["text"], "pong", "{call}");
}

#[tokio::test]
async fn mcp_rejects_a_missing_bearer_token() {
    let (cp, _services, _root, _dir) = boot().await;
    let resp = reqwest::Client::new()
        .post(cp.mcp_endpoint_url())
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// The primary stream's writer thread, which the in-memory project has.
async fn writer_thread(services: &oxplow_app::Services) -> oxplow_domain::Thread {
    use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
    let stream = services.stream_store.list().await.unwrap().pop().unwrap();
    services
        .thread_store
        .list_for_stream(&stream.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("primary stream has a writer thread")
}

/// A `run_command` over the wire runs as the session its bearer was
/// minted for: it's audited to that session's thread, and headers naming
/// another thread change nothing.
#[tokio::test]
async fn run_command_over_http_is_audited_to_the_bearers_thread() {
    let (cp, services, root, _dir) = boot().await;
    let url = cp.mcp_endpoint_url();
    let thread = writer_thread(&services).await;
    let token = common::bearer(&services, thread.id).await;
    let client = reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-oxplow-thread", "thr999".parse().unwrap());
            h.insert("x-oxplow-stream", "str999".parse().unwrap());
            h
        })
        .build()
        .unwrap();

    let (session, _) = post(
        &client,
        &url,
        &token,
        None,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "mcp-wire-test", "version": "0"}
            }
        }),
    )
    .await;
    let session = session.expect("server assigns an Mcp-Session-Id");
    post(
        &client,
        &url,
        &token,
        Some(&session),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    let (_, call) = post(
        &client,
        &url,
        &token,
        Some(&session),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "run_command", "arguments": {
                "id": "oxplow.config.set",
                "input": {"key": "zones", "value": [{"match": "src/**", "zone": "core"}]}
            }}
        }),
    )
    .await;
    let call = call.unwrap();
    assert_ne!(call["result"]["isError"], true, "{call}");
    let yaml = std::fs::read_to_string(root.join(".oxplow/project.yaml")).unwrap();
    assert!(yaml.contains("zone: core"), "{yaml}");
    let events = services.event_log_store.read_after(0, 20).await.unwrap();
    let executed = events
        .iter()
        .find(|e| e.envelope.event_type == "command.executed")
        .expect("command.executed logged");
    assert_eq!(executed.envelope.source, format!("agent:{}", thread.id));
    // The run is the bearer's session's: its audit row and its event say so.
    use oxplow_domain::stores::AgentSessionStore as _;
    let session = services
        .agent_session_store
        .list_open_for_thread(&thread.id)
        .await
        .unwrap()
        .pop()
        .unwrap()
        .id;
    assert_eq!(executed.envelope.anchors.agent_session_id, Some(session));
    let audit = oxplow_db::command_audit_store::SqliteCommandAuditStore::new(services.db.clone())
        .get(executed_audit_id(&executed.envelope))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.session_id, Some(session));
}

/// The audit row a `command.executed` names.
fn executed_audit_id(envelope: &oxplow_domain::events::Envelope) -> i64 {
    envelope.payload["audit_id"].as_i64().unwrap()
}

/// A bearer no session holds is refused before MCP sees the request.
#[tokio::test]
async fn mcp_rejects_a_bearer_of_no_session() {
    let (cp, _services, _root, _dir) = boot().await;
    let resp = reqwest::Client::new()
        .post(cp.mcp_endpoint_url())
        .bearer_auth("not-a-session-token")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}
