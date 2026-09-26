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
    let (cp, _services, _root, _dir) = boot().await;
    let url = cp.mcp_endpoint_url();
    let token = cp.hook_token.clone();
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
    assert!(init["result"]["instructions"]
        .as_str()
        .unwrap()
        .starts_with("Oxplow MCP server"));
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
    assert_ne!(tool("create_task")["annotations"]["readOnlyHint"], true);

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
