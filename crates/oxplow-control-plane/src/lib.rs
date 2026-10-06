//! In-process axum server that hosts the two surfaces the Claude Code
//! plugin needs to reach: hook delivery and the MCP protocol.
//!
//! Single TCP listener bound to `127.0.0.1:0` (ephemeral port). Two
//! routers:
//!
//! - `POST /hook/:event` — receives hook envelopes from the plugin's
//!   HTTP hooks, drains into [`oxplow_app::HookIngestService`].
//!   Bearer-auth via `Authorization: Bearer <hook_token>`.
//! - `POST /mcp` (and friends) — the rmcp Streamable HTTP transport
//!   wrapping [`oxplow_mcp::OxplowMcp`]. Same bearer token.
//!
//! Started once at boot from the Tauri main; the resulting
//! [`ControlPlane`] handle exposes `hook_base_url`, `mcp_endpoint_url`,
//! and `hook_token`, all of which the per-spawn plugin writer + agent-
//! command builder feed into env / config files.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{any_service, post},
    Json, Router,
};
use base64::Engine;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use oxplow_app::{HookEnvelope, Services, ToolDecision};
use oxplow_domain::{HookKind, StreamId, ThreadId};

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Returned by [`spawn`]. The Tauri main keeps this alive for the life
/// of the process — dropping it does not stop the server (background
/// task is detached), but the URLs/token in it are what the plugin
/// writer needs.
#[derive(Debug, Clone)]
pub struct ControlPlane {
    pub bind_addr: SocketAddr,
    pub hook_token: String,
}

impl ControlPlane {
    /// Absolute URL the plugin's HTTP hooks POST to. Event name is
    /// appended as a path segment, e.g. `<base>/PreToolUse`.
    pub fn hook_base_url(&self) -> String {
        format!("http://{}/hook", self.bind_addr)
    }

    /// Absolute URL Claude Code uses for the MCP HTTP transport.
    pub fn mcp_endpoint_url(&self) -> String {
        format!("http://{}/mcp", self.bind_addr)
    }

    /// Base URL for the OTLP metrics receiver (epic tsk22). This is the
    /// **base** the agent's OTEL exporter is pointed at via
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`; the SDK appends the signal path
    /// `/v1/metrics` (which [`handle_otlp_metrics`] serves). Codex, whose
    /// config wants the full signal URL, appends `/v1/metrics` itself.
    pub fn otlp_base_url(&self) -> String {
        format!("http://{}", self.bind_addr)
    }
}

#[derive(Clone)]
struct AppCtx {
    services: Arc<Services>,
    hook_token: Arc<String>,
}

/// Boot the control plane. Picks an ephemeral port on 127.0.0.1 and
/// returns immediately (the server runs in a detached tokio task).
pub async fn spawn(services: Arc<Services>) -> Result<ControlPlane, ControlPlaneError> {
    let token = generate_token();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = listener.local_addr()?;

    let ctx = AppCtx {
        services: services.clone(),
        hook_token: Arc::new(token.clone()),
    };

    let mcp_services = services.clone();
    let mcp_token = Arc::new(token.clone());

    // rmcp's StreamableHttpService is a tower::Service<Request>.
    // Mount it under /mcp via `any_service`. The factory closure runs
    // per-MCP-session to build a fresh OxplowMcp handler instance.
    let mcp_service = StreamableHttpService::new(
        move || Ok(oxplow_mcp::OxplowMcp::new(mcp_services.clone())),
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    // axum router for the MCP routes — wrap with our auth check.
    let mcp_auth_token = mcp_token.clone();
    let mcp_router = Router::new()
        .route_service("/mcp", any_service(mcp_service.clone()))
        .route_service("/mcp/", any_service(mcp_service))
        .layer(axum::middleware::from_fn(move |req, next| {
            let token = mcp_auth_token.clone();
            async move { auth_middleware(token, req, next).await }
        }));

    // Health-check endpoint. Not full dev-hot-reload (Rust dylib swap
    // in-process isn't practical with rmcp's tower service factory),
    // but lets external tooling verify the control plane is up + the
    // bearer token matches before spawning an agent.
    let dev_router = Router::new()
        .route("/dev/ping", post(handle_dev_ping))
        .layer(axum::middleware::from_fn({
            let token = mcp_token.clone();
            move |req, next| {
                let token = token.clone();
                async move { auth_middleware(token, req, next).await }
            }
        }));

    let hook_router = Router::new()
        .route("/hook/{event}", post(handle_hook))
        // OTLP receiver (epic tsk22). Agent CLIs export token usage here —
        // Claude as metrics, Codex as logs (its `response.completed` event) —
        // attribution rides the same X-Oxplow-* headers as hooks. Both signal
        // paths hit one handler; the ingest path decodes metrics-or-logs.
        .route("/v1/metrics", post(handle_otlp_metrics))
        .route("/v1/logs", post(handle_otlp_metrics))
        .with_state(ctx);

    let app = Router::new()
        .merge(hook_router)
        .merge(mcp_router)
        .merge(dev_router);

    info!(addr = %bind_addr, "control plane listening");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app.into_make_service()).await {
            warn!(?err, "control plane server exited");
        }
    });

    Ok(ControlPlane {
        bind_addr,
        hook_token: token,
    })
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Bearer auth check. Constant-time comparison via base64 round-trip
/// avoidance — token strings are random base64 of equal length, so a
/// straight `==` is fine.
async fn auth_middleware(
    expected_token: Arc<String>,
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    if !check_bearer(req.headers(), &expected_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }
    next.run(req).await
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let Some(auth) = headers.get(http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = auth.to_str() else {
        return false;
    };
    let Some(rest) = s.strip_prefix("Bearer ") else {
        return false;
    };
    rest == expected
}

async fn handle_dev_ping() -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "service": "oxplow-control-plane",
        })),
    )
        .into_response()
}

/// OTLP/HTTP metrics receiver (epic tsk22). Agent CLIs (Claude Code, Codex)
/// export token usage here. Attribution reuses the hook spine: the owning
/// thread rides the `X-Oxplow-Thread` header the spawn path injects into the
/// exporter (one agent process per thread, so it is constant for its
/// lifetime). The body is logged as one `agent.tokens.reported` event
/// (`oxplow_app::otlp_ingest`); the `token_usage.otlp` consumer counts it.
///
/// Always answers 200 (an empty OTLP success ack): token capture is a
/// best-effort side-band, and a non-2xx would make the exporter retry-storm a
/// payload we can't use. Missing attribution headers → accept + drop.
async fn handle_otlp_metrics(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_bearer(&headers, &ctx.hook_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }
    // Opt-in wire-format diagnostic (tsk25): when `OXPLOW_OTLP_DEBUG` names a
    // file, append a human-readable dump of every received export to it — used
    // to discover an agent's real OTEL shape (e.g. Codex) from a live run.
    // Off by default, zero cost when unset.
    otlp_debug_dump(&headers, &body);
    let thread_id = headers
        .get("x-oxplow-thread")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .and_then(ThreadId::try_from_str);
    let Some(thread_id) = thread_id else {
        warn!("OTLP export missing the X-Oxplow-Thread header; dropping");
        return otlp_ok();
    };
    // Logged as `agent.tokens.reported`; the `token_usage.otlp` consumer
    // counts it.
    match ctx.services.otlp_ingest.ingest(thread_id, &body).await {
        Ok(logged) => tracing::debug!(logged, "OTLP token export"),
        Err(err) => warn!(?err, "failed to log OTLP token export"),
    }
    otlp_ok()
}

/// Append a human-readable dump of an OTLP export to the file named by
/// `OXPLOW_OTLP_DEBUG` (tsk25). No-op when the env var is unset. Best-effort:
/// a file/IO error is ignored (it's a diagnostic, never load-bearing).
fn otlp_debug_dump(headers: &HeaderMap, body: &[u8]) {
    let Ok(path) = std::env::var("OXPLOW_OTLP_DEBUG") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let hdr = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>")
    };
    let entry = format!(
        "=== OTLP export ({} bytes) thread={} ===\n{}\n\n",
        body.len(),
        hdr("x-oxplow-thread"),
        oxplow_app::otlp_tokens::summarize_metrics_request(body),
    );
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(entry.as_bytes());
    }
}

/// Append every hook payload, as the agent sent it, to the file named by
/// `OXPLOW_HOOK_DEBUG`: how to learn a harness's real payload shapes (the
/// Stop's final message, subagent ids, its own task list) from a live run
/// before depending on them. No-op when unset; best-effort, like
/// [`otlp_debug_dump`].
fn hook_debug_dump(event: &str, thread: Option<&str>, body: &[u8]) {
    let Ok(path) = std::env::var("OXPLOW_HOOK_DEBUG") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(hook_debug_entry(event, thread, body).as_bytes());
    }
}

/// One JSON line: the event, the thread its headers named, when, and the
/// payload (parsed when it's JSON, else the text as sent).
fn hook_debug_entry(event: &str, thread: Option<&str>, body: &[u8]) -> String {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(body).into_owned()));
    let entry = serde_json::json!({
        "event": event,
        "thread": thread,
        "at": oxplow_domain::Timestamp::now().to_string(),
        "payload": payload,
    });
    format!("{entry}\n")
}

/// Empty OTLP success ack: a 200 with a protobuf content type and an empty
/// body, which deserializes to an empty `ExportMetricsServiceResponse` — what
/// OTLP exporters expect for a successful export.
fn otlp_ok() -> Response {
    (
        StatusCode::OK,
        [(http::header::CONTENT_TYPE, "application/x-protobuf")],
        axum::body::Bytes::new(),
    )
        .into_response()
}

/// Upper bound on hook decision time. Claude Code blocks on the hook
/// response, so a wedged backend (DB writer held by a snapshot flush,
/// a slow store query) must not stall the agent indefinitely. On
/// expiry we return the generic ack — i.e. allow the tool call / emit
/// no directive. Availability over enforcement: a missed deny on one
/// pathological turn beats a frozen agent, and the MCP tools re-check
/// write-guard + filing at the call site anyway.
const HOOK_HANDLING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn handle_hook(
    State(ctx): State<AppCtx>,
    AxumPath(event): AxumPath<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_bearer(&headers, &ctx.hook_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }
    let event_name = event.clone();
    bounded_hook_response(
        HOOK_HANDLING_TIMEOUT,
        &event_name,
        handle_hook_inner(ctx, event, headers, body),
    )
    .await
}

/// Race `fut` against `timeout`; on expiry, log and fall back to the
/// generic ack (allow / no directive). Split from [`handle_hook`] so
/// the timeout path is unit-testable with a never-resolving future.
async fn bounded_hook_response<F>(timeout: std::time::Duration, event: &str, fut: F) -> Response
where
    F: std::future::Future<Output = Response>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(resp) => resp,
        Err(_) => {
            warn!(
                event,
                timeout_ms = timeout.as_millis() as u64,
                "hook handling timed out — returning default allow/ack so the agent isn't stalled"
            );
            hook_ack()
        }
    }
}

async fn handle_hook_inner(
    ctx: AppCtx,
    event: String,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let stream_id = headers
        .get("x-oxplow-stream")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .and_then(StreamId::try_from_str);
    let thread_id = headers
        .get("x-oxplow-thread")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .and_then(ThreadId::try_from_str);

    hook_debug_dump(&event, thread_id.map(|t| t.to_string()).as_deref(), &body);

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "non-utf8 body").into_response();
        }
    };

    let body_value: Option<serde_json::Value> = serde_json::from_str(&body_str).ok();
    let session_id = body_value
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    // SessionStart / SessionEnd: the ingest tracks the session (the
    // thread's resume id, `agent.session.*`; a `/clear` of the resume
    // session drops it — see `hook_ingest`). A startup/resume/clear/compact
    // also gives the agent a fresh system prompt, so discard the context
    // baselines and let the next prompt inject one fresh block.
    if event == "SessionStart" || event == "SessionEnd" {
        if event == "SessionStart" {
            ctx.services
                .agent_context
                .reset_session(session_id.as_deref());
        }
        let kind = if event == "SessionStart" {
            HookKind::SessionStart
        } else {
            HookKind::SessionEnd
        };
        let envelope = HookEnvelope {
            kind,
            thread_id,
            stream_id,
            session_id,
            payload_json: body_str,
            prompt: None,
            decision: None,
        };
        if let Err(err) = ctx.services.hook_ingest.ingest(envelope).await {
            warn!(?event, ?err, "hook ingest failed");
        }
        return hook_ack();
    }

    let kind = match parse_hook_kind(&event) {
        Some(k) => k,
        None => {
            // Unknown but non-fatal — record nothing, ack so the agent
            // doesn't block.
            return hook_ack();
        }
    };

    let prompt = if kind == HookKind::UserPromptSubmit {
        body_value
            .as_ref()
            .and_then(|v| v.get("prompt"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    // PreToolUse — runs BEFORE ingest so denial returns immediately
    // and the persisted record reflects what actually happened.
    if kind == HookKind::PreToolUse {
        if let Some(reason) = pre_tool_check(&ctx, thread_id.as_ref(), body_value.as_ref()).await {
            // Logged with the policy's decision, so the record shows what
            // the runtime did.
            let envelope = HookEnvelope {
                kind,
                thread_id,
                stream_id,
                session_id: session_id.clone(),
                payload_json: body_str,
                prompt: None,
                decision: Some(ToolDecision {
                    allowed: false,
                    reason: Some(reason.clone()),
                }),
            };
            if let Err(err) = ctx.services.hook_ingest.ingest(envelope).await {
                warn!(?err, "hook ingest failed for a denied tool call");
            }
            return (StatusCode::OK, Json(pre_tool_deny(reason))).into_response();
        }
    }

    let envelope = HookEnvelope {
        kind,
        thread_id,
        stream_id,
        session_id,
        payload_json: body_str,
        prompt,
        decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
            allowed: true,
            reason: None,
        }),
    };

    let envelope_for_resume = envelope.clone();
    let ingested = match ctx.services.hook_ingest.ingest(envelope).await {
        Ok(outcome) => outcome,
        Err(err) => {
            // The agent can't act on an error status — Claude Code just
            // prints a "non-blocking status code" warning into the user's
            // terminal. Log the cause server-side and ack anyway.
            warn!(?event, ?err, "hook ingest failed");
            return hook_ack();
        }
    };
    // What the turn this Stop closed did (its own tool events); none when
    // no turn was open.
    let turn_signals = match ingested.closed_turn {
        Some(turn) => oxplow_app::agent_policy::TurnSignals::of_turn(&ctx.services.db, turn)
            .await
            .ok(),
        None => None,
    };

    // Token usage (tsk104) is counted from the Stop's `agent.turn.ended`
    // by the `token_usage.turns` pump reactor (P3.7), not in the hook.

    // PostToolUse: record the call (wiki attribution, effort claim, tool
    // call, collection — pump reactors on the event the ingest logged) and
    // hand back any context for the agent (the ROLE CHANGE banner after
    // ExitPlanMode, else the thread's undelivered nudges).
    if kind == HookKind::PostToolUse {
        if let (Some(thread_id), Some(body)) =
            (envelope_for_resume.thread_id.as_ref(), body_value.as_ref())
        {
            if let Some(context) = ctx
                .services
                .agent_context
                .post_tool_context(
                    &ctx.services,
                    thread_id,
                    envelope_for_resume.session_id.as_deref(),
                    body,
                )
                .await
            {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "hookSpecificOutput": {
                            "hookEventName": "PostToolUse",
                            "additionalContext": context,
                        }
                    })),
                )
                    .into_response();
            }
        }
    }

    // UserPromptSubmit: session context (when it changed), prompt
    // advisories and the effort's decisions ride additionalContext.
    if kind == HookKind::UserPromptSubmit {
        if let Some(thread_id) = envelope_for_resume.thread_id.as_ref() {
            if let Some(combined) = ctx
                .services
                .agent_context
                .prompt_context(
                    &ctx.services,
                    thread_id,
                    envelope_for_resume.session_id.as_deref(),
                )
                .await
            {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "hookSpecificOutput": {
                            "hookEventName": "UserPromptSubmit",
                            "additionalContext": combined,
                        }
                    })),
                )
                    .into_response();
            }
        }
    }

    // Stop — emit a directive after the turn closes when the
    // in_progress audit branch (or filed-but-didn't-ship advisory)
    // fires. The turn's signals (activity, writes, awaiting the person,
    // a subagent still running) are read from the events anchored to the
    // turn the Stop closed (`TurnSignals::of_turn`), after the ingest.
    if kind == HookKind::Stop {
        if let Some(directive) =
            stop_directive(&ctx, thread_id.as_ref(), turn_signals.as_ref()).await
        {
            return (StatusCode::OK, Json(directive)).into_response();
        }
    }

    hook_ack()
}

/// The default no-op hook acknowledgement. MUST be `200 {}` — Claude
/// Code's HTTP hooks treat any other status (including an empty 202)
/// as a failure and print a "Failed with non-blocking status code"
/// warning into the user's terminal, which fills the xterm with noise
/// on Edit/Write-heavy turns. See `.context/agent-model.md`.
fn hook_ack() -> Response {
    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}

/// Claude's `hookSpecificOutput` refusing a PreToolUse for `reason`.
fn pre_tool_deny(reason: String) -> serde_json::Value {
    use oxplow_runtime::write_guard::{HookSpecificOutput, WriteGuardDeny};
    serde_json::to_value(WriteGuardDeny {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: reason,
        },
    })
    .unwrap_or_default()
}

/// Run the shared agent policy (write guard, then filing) against the
/// PreToolUse payload. `Some(reason)` refuses; `None` allows. Tools
/// neither rule can refuse skip the policy's I/O (`claude_intent`
/// returns `None` for them).
async fn pre_tool_check(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    body: Option<&serde_json::Value>,
) -> Option<String> {
    use oxplow_runtime::policy::{PolicyDecision, ToolIntent};
    let intent = oxplow_app::agent_policy::claude_intent(body?)?;
    let decision = ctx
        .services
        .agent_policy
        .check_tool(
            &ctx.services,
            thread_id?,
            &ToolIntent {
                label: &intent.label,
                kind: intent.kind,
                paths: &intent.paths,
            },
        )
        .await;
    match decision {
        PolicyDecision::Allow => None,
        PolicyDecision::Deny { reason, .. } => Some(reason),
    }
}

/// The shared policy's end-of-turn directive, rendered as Claude's Stop
/// `{decision: "block", reason}`.
async fn stop_directive(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    turn_signals: Option<&oxplow_app::agent_policy::TurnSignals>,
) -> Option<serde_json::Value> {
    let directive = ctx
        .services
        .agent_policy
        .on_turn_end(&ctx.services, thread_id?, turn_signals)
        .await?;
    serde_json::to_value(directive).ok()
}

fn parse_hook_kind(event: &str) -> Option<HookKind> {
    match event {
        "PreToolUse" => Some(HookKind::PreToolUse),
        "PostToolUse" => Some(HookKind::PostToolUse),
        "UserPromptSubmit" => Some(HookKind::UserPromptSubmit),
        "Stop" => Some(HookKind::Stop),
        // SessionStart / SessionEnd are routed before this (they carry
        // no policy); anything else (Notification, …) is acked unread.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_hook_response_passes_through_fast_futures() {
        let resp = bounded_hook_response(std::time::Duration::from_secs(1), "Stop", async {
            (StatusCode::OK, "directive").into_response()
        })
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bounded_hook_response_falls_back_to_ack_on_timeout() {
        let resp = bounded_hook_response(
            std::time::Duration::from_millis(10),
            "PreToolUse",
            std::future::pending::<Response>(),
        )
        .await;
        // Safe default: allow / no directive — never stall the agent.
        // Must be 200 (not 202): Claude Code prints a "non-blocking
        // status code" warning into the terminal on any other status.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn token_is_long_enough() {
        let t = generate_token();
        // 32 bytes base64-url-no-pad → 43 chars.
        assert_eq!(t.len(), 43);
    }

    #[test]
    fn bearer_check_accepts_matching() {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer abc"),
        );
        assert!(check_bearer(&h, "abc"));
    }

    #[test]
    fn bearer_check_rejects_missing() {
        assert!(!check_bearer(&HeaderMap::new(), "abc"));
    }

    #[test]
    fn bearer_check_rejects_wrong() {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer xyz"),
        );
        assert!(!check_bearer(&h, "abc"));
    }

    /// `OXPLOW_HOOK_DEBUG`'s entry is one JSON line per hook: the event,
    /// its headers' thread, and the payload as sent (kept verbatim when it
    /// isn't JSON).
    #[test]
    fn a_hook_debug_entry_is_one_json_line_with_the_payload() {
        let line = hook_debug_entry(
            "Stop",
            Some("thr3"),
            br#"{"session_id":"s","last_assistant_message":"done"}"#,
        );
        assert!(
            line.ends_with('\n') && line.matches('\n').count() == 1,
            "{line:?}"
        );
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["event"], "Stop");
        assert_eq!(v["thread"], "thr3");
        assert_eq!(v["payload"]["last_assistant_message"], "done");
        assert!(v["at"].as_str().is_some());
        let raw = hook_debug_entry("Notification", None, b"not json");
        let v: serde_json::Value = serde_json::from_str(raw.trim_end()).unwrap();
        assert_eq!(v["payload"], "not json");
        assert!(v["thread"].is_null());
    }

    #[test]
    fn parse_hook_kind_known() {
        assert!(matches!(
            parse_hook_kind("PreToolUse"),
            Some(HookKind::PreToolUse)
        ));
        assert!(matches!(parse_hook_kind("Stop"), Some(HookKind::Stop)));
    }

    #[test]
    fn parse_hook_kind_unknown_returns_none() {
        assert!(parse_hook_kind("SessionStart").is_none());
        assert!(parse_hook_kind("garbage").is_none());
    }

    #[test]
    fn parse_hook_kind_covers_each_known_kind() {
        assert!(matches!(
            parse_hook_kind("PreToolUse"),
            Some(HookKind::PreToolUse)
        ));
        assert!(matches!(
            parse_hook_kind("PostToolUse"),
            Some(HookKind::PostToolUse)
        ));
        assert!(matches!(
            parse_hook_kind("UserPromptSubmit"),
            Some(HookKind::UserPromptSubmit)
        ));
        assert!(matches!(parse_hook_kind("Stop"), Some(HookKind::Stop)));
        assert!(parse_hook_kind("").is_none());
        assert!(parse_hook_kind("PRETOOLUSE").is_none()); // case-sensitive
    }

    #[test]
    fn bearer_check_rejects_malformed_header() {
        // No "Bearer " prefix — even if the token bytes match.
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("abc"),
        );
        assert!(!check_bearer(&h, "abc"));
    }

    #[test]
    fn bearer_check_is_case_sensitive_on_scheme() {
        // "bearer " (lowercase) is rejected — clients must send the
        // canonical "Bearer " scheme.
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("bearer abc"),
        );
        assert!(!check_bearer(&h, "abc"));
    }

    #[test]
    fn generated_tokens_are_unique() {
        // Sanity: the OS RNG produces distinct tokens across calls.
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }
}
