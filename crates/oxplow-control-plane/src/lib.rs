//! In-process axum server that hosts the two surfaces the Claude Code
//! plugin needs to reach: hook delivery and the MCP protocol.
//!
//! Single TCP listener bound to `127.0.0.1:0` (ephemeral port). Two
//! routers:
//!
//! - `POST /hook/:event` — receives hook envelopes from the plugin's
//!   HTTP hooks, drains into [`oxplow_app::HookIngestService`].
//! - `POST /v1/metrics`, `/v1/logs` — the agents' OTLP exports.
//! - `POST /mcp` (and friends) — the rmcp Streamable HTTP transport
//!   wrapping [`oxplow_mcp::OxplowMcp`].
//!
//! Every route takes `Authorization: Bearer <token>`, the token minted for
//! one agent session at its launch ([`oxplow_app::session_auth`]). The
//! bearer is the whole of who a request comes from: its session, thread,
//! stream and harness. Nothing else a request carries names its sender.
//!
//! Started once at boot by the daemon; the resulting [`ControlPlane`]
//! handle exposes the URLs the launch feeds into each agent's env and
//! config files.

pub mod hook_client;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{any_service, post},
    Extension, Json, Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use oxplow_app::session_auth::Principal;
use oxplow_app::{HookEnvelope, Services, ToolDecision};
use oxplow_domain::agent::observe::{HookAnswer, Prompt};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::agent::tool::ToolUse;
use oxplow_domain::{HookKind, ThreadId};

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Returned by [`spawn`]. The daemon keeps this alive for the life of
/// the process — dropping it does not stop the server (background task
/// is detached), but the URLs in it are what a launch needs.
#[derive(Debug, Clone)]
pub struct ControlPlane {
    pub bind_addr: SocketAddr,
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
}

/// Boot the control plane. Picks an ephemeral port on 127.0.0.1 and
/// returns immediately (the server runs in a detached tokio task).
pub async fn spawn(services: Arc<Services>) -> Result<ControlPlane, ControlPlaneError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = listener.local_addr()?;

    let ctx = AppCtx {
        services: services.clone(),
    };

    let mcp_services = services.clone();

    // rmcp's StreamableHttpService is a tower::Service<Request>.
    // Mount it under /mcp via `any_service`. The factory closure runs
    // per-MCP-session to build a fresh OxplowMcp handler instance.
    let mcp_service = StreamableHttpService::new(
        move || Ok(oxplow_mcp::OxplowMcp::new(mcp_services.clone())),
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    // The MCP routes, behind the session's bearer: the middleware puts its
    // `Principal` in the request's extensions, where rmcp hands it to the
    // tools (`oxplow_mcp::caller_of`).
    let mcp_router = Router::new()
        .route_service("/mcp", any_service(mcp_service.clone()))
        .route_service("/mcp/", any_service(mcp_service))
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth_middleware,
        ));

    // Health-check endpoint: lets external tooling verify the control
    // plane is up and a session's bearer is live.
    let dev_router = Router::new()
        .route("/dev/ping", post(handle_dev_ping))
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth_middleware,
        ));

    let hook_router = Router::new()
        .route("/hook/{event}", post(handle_hook))
        // OTLP receiver (epic tsk22). Agent CLIs export token usage here —
        // Claude as metrics, Codex as logs (its `response.completed` event) —
        // attributed by the bearer like a hook. Both signal paths hit one
        // handler; the ingest path decodes metrics-or-logs.
        .route("/v1/metrics", post(handle_otlp_metrics))
        .route("/v1/logs", post(handle_otlp_metrics))
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth_middleware,
        ))
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

    Ok(ControlPlane { bind_addr })
}

/// Admit a request only with a live session's bearer, and hand the
/// handler that session's [`Principal`] in the request's extensions.
async fn auth_middleware(
    State(ctx): State<AppCtx>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let Some(principal) =
        bearer(req.headers()).and_then(|t| ctx.services.session_auth.authenticate(t))
    else {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    };
    req.extensions_mut().insert(principal);
    next.run(req).await
}

/// The token of an `Authorization: Bearer <token>` header (the scheme as
/// written, case-sensitively).
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
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
/// export token usage here, attributed to the session whose bearer the
/// exporter carries. The body is logged as one `agent.tokens.reported`
/// event (`oxplow_app::otlp_ingest`); the `token_usage.otlp` consumer
/// counts it.
///
/// Always answers 200 (an empty OTLP success ack): token capture is a
/// best-effort side-band, and a non-2xx would make the exporter retry-storm a
/// payload we can't use.
async fn handle_otlp_metrics(
    State(ctx): State<AppCtx>,
    Extension(principal): Extension<Principal>,
    body: axum::body::Bytes,
) -> Response {
    // Opt-in wire-format diagnostic (tsk25): when `OXPLOW_OTLP_DEBUG` names a
    // file, append a human-readable dump of every received export to it — used
    // to discover an agent's real OTEL shape (e.g. Codex) from a live run.
    // Off by default, zero cost when unset.
    otlp_debug_dump(&principal, &body);
    match ctx
        .services
        .otlp_ingest
        .ingest(principal.thread, Some(principal.session), &body)
        .await
    {
        Ok(logged) => tracing::debug!(logged, "OTLP token export"),
        Err(err) => warn!(?err, "failed to log OTLP token export"),
    }
    otlp_ok()
}

/// Append a human-readable dump of an OTLP export to the file named by
/// `OXPLOW_OTLP_DEBUG` (tsk25). No-op when the env var is unset. Best-effort:
/// a file/IO error is ignored (it's a diagnostic, never load-bearing).
fn otlp_debug_dump(principal: &Principal, body: &[u8]) {
    let Ok(path) = std::env::var("OXPLOW_OTLP_DEBUG") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let entry = format!(
        "=== OTLP export ({} bytes) thread={} ===\n{}\n\n",
        body.len(),
        principal.thread,
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
/// expiry we return the ack — i.e. allow the tool call.
/// Availability over enforcement: a missed deny on one pathological turn
/// beats a frozen agent, and the MCP tools re-check the write guard at
/// the call site anyway.
const HOOK_HANDLING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn handle_hook(
    State(ctx): State<AppCtx>,
    Extension(principal): Extension<Principal>,
    AxumPath(event): AxumPath<String>,
    body: axum::body::Bytes,
) -> Response {
    let event_name = event.clone();
    // A timed-out hook gets its harness's ack.
    let ack = respond(
        &ctx.services.harnesses,
        Some(&principal.harness),
        &HookAnswer::Ack,
    )
    .await;
    bounded_hook_response(
        HOOK_HANDLING_TIMEOUT,
        &event_name,
        ack,
        handle_hook_inner(ctx, principal, event, body),
    )
    .await
}

/// Race `fut` against `timeout`; on expiry, log and fall back to `ack`
/// (allow / no directive). Split from [`handle_hook`] so the timeout path
/// is unit-testable with a never-resolving future.
async fn bounded_hook_response<F>(
    timeout: std::time::Duration,
    event: &str,
    ack: Response,
    fut: F,
) -> Response
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
            ack
        }
    }
}

async fn handle_hook_inner(
    ctx: AppCtx,
    principal: Principal,
    event: String,
    body: axum::body::Bytes,
) -> Response {
    // Who sent it is the bearer's session; nothing in the request says.
    let thread_id = Some(principal.thread);
    let stream_id = Some(principal.stream);
    let agent_session_id = Some(principal.session);

    hook_debug_dump(&event, Some(&principal.thread.to_string()), &body);

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
    // Notification: a permission prompt waits on the person (the ingest
    // records it as the thread's status). SubagentStart / SubagentStop:
    // the subagent the bearer's harness reads in the body. None of these
    // carry policy.
    if matches!(
        event.as_str(),
        "SessionStart" | "SessionEnd" | "Notification" | "SubagentStart" | "SubagentStop"
    ) {
        if event == "SessionStart" {
            ctx.services
                .agent_context
                .reset_session(session_id.as_deref());
        }
        let kind = match event.as_str() {
            "SessionStart" => HookKind::SessionStart,
            "SessionEnd" => HookKind::SessionEnd,
            "SubagentStart" => HookKind::SubagentStart,
            "SubagentStop" => HookKind::SubagentStop,
            _ => HookKind::Notification,
        };
        let subagent = match (kind, body_value.as_ref()) {
            (HookKind::SubagentStart | HookKind::SubagentStop, Some(body)) => {
                match ctx.services.harnesses.get(&principal.harness) {
                    Ok(h) => h.subagent(body).await,
                    Err(_) => None,
                }
            }
            _ => None,
        };
        let envelope = HookEnvelope {
            kind,
            thread_id,
            stream_id,
            agent_session_id,
            session_id,
            payload_json: body_str,
            prompt: None,
            decision: None,
            tool: None,
            subagent,
        };
        let harness = match ctx.services.hook_ingest.ingest(envelope).await {
            Ok(outcome) => outcome.harness,
            Err(err) => {
                warn!(?event, ?err, "hook ingest failed");
                None
            }
        };
        return respond(
            &ctx.services.harnesses,
            harness.as_deref(),
            &HookAnswer::Ack,
        )
        .await;
    }

    let kind = match parse_hook_kind(&event) {
        Some(k) => k,
        None => {
            // Unknown but non-fatal — record nothing, ack so the agent
            // doesn't block.
            return respond(&ctx.services.harnesses, None, &HookAnswer::Ack).await;
        }
    };

    // What a prompt hook is, as the bearer's harness reads it: a person's
    // prompt, or a subagent handing its report back (no prompt; the
    // subagent).
    let (prompt, handback) = match (kind, body_value.as_ref()) {
        (HookKind::UserPromptSubmit, Some(body)) => {
            match ctx.services.harnesses.get(&principal.harness) {
                Ok(h) => match h.prompt(body).await {
                    Some(Prompt::Person { text }) => (Some(text), None),
                    Some(Prompt::Handback { subagent }) => (None, Some(subagent)),
                    None => (None, None),
                },
                Err(_) => (None, None),
            }
        }
        _ => (None, None),
    };

    // A tool hook's call in oxplow's vocabulary, as the bearer's harness
    // maps its body: what the policy, the ingest and the context read.
    let tool = match (kind, body_value.as_ref()) {
        (HookKind::PreToolUse | HookKind::PostToolUse, Some(body)) => {
            match ctx.services.harnesses.get(&principal.harness) {
                Ok(h) => h.tool_use(body).await,
                Err(_) => None,
            }
        }
        _ => None,
    };

    // PreToolUse — runs BEFORE ingest so denial returns immediately
    // and the persisted record reflects what actually happened.
    if kind == HookKind::PreToolUse {
        if let Some(reason) = pre_tool_check(&ctx, principal.thread, tool.as_ref()).await {
            // Logged with the policy's decision, so the record shows what
            // the runtime did.
            let envelope = HookEnvelope {
                kind,
                thread_id,
                stream_id,
                agent_session_id,
                session_id: session_id.clone(),
                payload_json: body_str,
                prompt: None,
                decision: Some(ToolDecision {
                    allowed: false,
                    reason: Some(reason.clone()),
                }),
                tool: tool.clone(),
                subagent: None,
            };
            let harness = match ctx.services.hook_ingest.ingest(envelope).await {
                Ok(outcome) => outcome.harness,
                Err(err) => {
                    warn!(?err, "hook ingest failed for a denied tool call");
                    None
                }
            };
            return respond(
                &ctx.services.harnesses,
                harness.as_deref(),
                &HookAnswer::Deny { reason },
            )
            .await;
        }
    }

    let envelope = HookEnvelope {
        kind,
        thread_id,
        stream_id,
        agent_session_id,
        session_id,
        payload_json: body_str,
        prompt,
        decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
            allowed: true,
            reason: None,
        }),
        tool: tool.clone(),
        subagent: handback.clone(),
    };

    let envelope_for_resume = envelope.clone();
    let harness = match ctx.services.hook_ingest.ingest(envelope).await {
        Ok(outcome) => outcome.harness,
        Err(err) => {
            // The agent can't act on an error status — Claude Code just
            // prints a "non-blocking status code" warning into the user's
            // terminal. Log the cause server-side and ack anyway.
            warn!(?event, ?err, "hook ingest failed");
            return respond(&ctx.services.harnesses, None, &HookAnswer::Ack).await;
        }
    };
    let harness = harness.as_deref();
    // Token usage (tsk104) is counted from the Stop's `agent.turn.ended`
    // by the `token_usage.turns` pump reactor (P3.7), not in the hook.

    // PostToolUse: record the call (wiki attribution, effort claim, tool
    // call, collection — pump reactors on the event the ingest logged) and
    // hand back any context for the agent (the ROLE CHANGE banner after a
    // plan settles, else the thread's undelivered nudges).
    if kind == HookKind::PostToolUse {
        if let Some(thread_id) = envelope_for_resume.thread_id.as_ref() {
            if let Some(context) = ctx
                .services
                .agent_context
                .post_tool_context(
                    &ctx.services,
                    thread_id,
                    envelope_for_resume.session_id.as_deref(),
                    tool.as_ref(),
                )
                .await
            {
                return respond(
                    &ctx.services.harnesses,
                    harness,
                    &HookAnswer::Context {
                        event: HookKind::PostToolUse,
                        text: context,
                    },
                )
                .await;
            }
        }
    }

    // UserPromptSubmit: session context (when it changed), prompt
    // advisories and the effort's decisions ride additionalContext.
    if kind == HookKind::UserPromptSubmit && handback.is_none() {
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
                return respond(
                    &ctx.services.harnesses,
                    harness,
                    &HookAnswer::Context {
                        event: HookKind::UserPromptSubmit,
                        text: combined,
                    },
                )
                .await;
            }
        }
    }

    // Stop is never refused: the ingest closed the turn; the ack ends it
    // (.context/work-tracking.md "No gates").
    respond(&ctx.services.harnesses, harness, &HookAnswer::Ack).await
}

/// `answer` as the hook's harness renders it (`AgentHarness::render`): its
/// session's, else the default harness's; an empty object with none
/// registered. Always `200` — Claude Code's HTTP hooks treat any other
/// status (including an empty 202) as a failure and print a "Failed with
/// non-blocking status code" warning into the user's terminal, which fills
/// the xterm with noise on Edit/Write-heavy turns. See
/// `.context/agent-model.md`.
async fn respond(
    harnesses: &HarnessRegistry,
    harness: Option<&str>,
    answer: &HookAnswer,
) -> Response {
    let body = match harness
        .and_then(|h| harnesses.get(h).ok())
        .or_else(|| harnesses.default().ok())
    {
        Some(h) => h.render(answer).await,
        None => serde_json::json!({}),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Run the shared agent policy (the write guard) against the call.
/// `Some(reason)` refuses; `None` allows. A call the policy can't refuse
/// (anything but an edit) skips its I/O.
async fn pre_tool_check(ctx: &AppCtx, thread: ThreadId, tool: Option<&ToolUse>) -> Option<String> {
    use oxplow_runtime::policy::PolicyDecision;
    let tool = tool.filter(|t| oxplow_app::agent_policy::may_refuse(t))?;
    let decision = ctx
        .services
        .agent_policy
        .check_tool(
            &ctx.services,
            &thread,
            &oxplow_app::agent_policy::intent_of(tool),
        )
        .await;
    match decision {
        PolicyDecision::Allow => None,
        PolicyDecision::Deny { reason, .. } => Some(reason),
    }
}

fn parse_hook_kind(event: &str) -> Option<HookKind> {
    match event {
        "PreToolUse" => Some(HookKind::PreToolUse),
        "PostToolUse" => Some(HookKind::PostToolUse),
        "UserPromptSubmit" => Some(HookKind::UserPromptSubmit),
        "Stop" => Some(HookKind::Stop),
        // SessionStart / SessionEnd / Notification are routed before this
        // (they carry no policy); anything else is acked unread.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_hook_response_passes_through_fast_futures() {
        let resp = bounded_hook_response(
            std::time::Duration::from_secs(1),
            "Stop",
            StatusCode::NO_CONTENT.into_response(),
            async { (StatusCode::OK, "directive").into_response() },
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bounded_hook_response_falls_back_to_ack_on_timeout() {
        let none = HarnessRegistry::new(std::sync::Arc::new(String::new));
        let resp = bounded_hook_response(
            std::time::Duration::from_millis(10),
            "PreToolUse",
            respond(&none, None, &HookAnswer::Ack).await,
            std::future::pending::<Response>(),
        )
        .await;
        // Safe default: allow / no directive — never stall the agent.
        // Must be 200 (not 202): Claude Code prints a "non-blocking
        // status code" warning into the terminal on any other status.
        assert_eq!(resp.status(), StatusCode::OK);
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

    fn auth(value: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static(value),
        );
        h
    }

    #[test]
    fn the_bearer_is_the_token_after_the_scheme() {
        assert_eq!(bearer(&auth("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&HeaderMap::new()), None);
        // No scheme, or one in another case: clients send "Bearer ".
        assert_eq!(bearer(&auth("abc")), None);
        assert_eq!(bearer(&auth("bearer abc")), None);
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
}
