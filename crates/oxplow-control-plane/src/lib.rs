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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
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
use parking_lot::Mutex;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use oxplow_app::{
    build_session_context_block_with_role, role_change_banner, HookEnvelope, RoleMode, Services,
};
use oxplow_domain::stores::{AgentTurnStore, StreamStore, ThreadStore};
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

/// Captures the thread's writer/read-only role on the FIRST hook the
/// runtime sees for a given agent session id, then reuses that
/// snapshot as the comparison baseline for the ROLE CHANGE banner on
/// every subsequent hook of that session. Keyed by session_id (not
/// thread_id) so a thread that re-attaches with a fresh agent
/// session — e.g. after a daemon restart — gets a fresh baseline.
/// Loss across restart is acceptable: the worst case is one extra
/// no-op turn before the next promotion gets a banner.
#[derive(Default)]
struct RoleState {
    initial_role_by_session_id: HashMap<String, RoleMode>,
    /// Last `<session-context>` block returned for each session. The
    /// launch prompt already carries this data; hook injection is for
    /// refreshing mutable values, so byte-identical repeats add noise
    /// without giving the agent new information.
    last_context_by_session_id: HashMap<String, String>,
}

#[derive(Clone)]
struct AppCtx {
    services: Arc<Services>,
    hook_token: Arc<String>,
    role_state: Arc<Mutex<RoleState>>,
    /// Last resume session_id the runtime believes is persisted per
    /// thread. The resume tracker fires on EVERY hook but the session
    /// id only changes once per session, so this lets repeated hooks
    /// skip the `thread_store.get` + upsert entirely. A stale entry only
    /// ever costs one extra DB read (never wrong behavior), so losing it
    /// across a daemon restart is fine.
    resume_state: Arc<Mutex<HashMap<ThreadId, String>>>,
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
        role_state: Arc::new(Mutex::new(RoleState::default())),
        resume_state: Arc::new(Mutex::new(HashMap::new())),
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
/// export token-usage metrics here. Attribution reuses the hook spine: the
/// owning thread/stream ride the `X-Oxplow-Thread`/`X-Oxplow-Stream` headers the
/// spawn path injects into the exporter (one agent process per thread, so the
/// headers are constant for its lifetime). The protobuf body is decoded +
/// projected onto `oxplow.tokens` facts by the token-usage service.
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
    let (Some(stream_id), Some(thread_id)) = (stream_id, thread_id) else {
        warn!("OTLP metrics export missing X-Oxplow-Thread/Stream headers; dropping");
        return otlp_ok();
    };
    match ctx
        .services
        .token_usage
        .ingest_otlp_tokens(&thread_id, &stream_id, &body)
        .await
    {
        Ok(n) => tracing::debug!(facts = n, "ingested OTLP token export"),
        Err(err) => warn!(?err, "failed to ingest OTLP token export"),
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
        "=== OTLP export ({} bytes) thread={} stream={} ===\n{}\n\n",
        body.len(),
        hdr("x-oxplow-thread"),
        hdr("x-oxplow-stream"),
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

    // SessionStart is runtime state, not a persisted domain hook. A
    // startup/resume/clear/compact gives the agent a fresh system
    // prompt, so discard both comparison baselines and let the next
    // UserPromptSubmit inject one fresh block for the new context.
    if event == "SessionStart" {
        reset_session_context_state(&ctx.role_state, session_id.as_deref());
        return hook_ack();
    }

    // SessionEnd: `/clear` ends the session and Claude Code starts a
    // fresh one WITHOUT any HTTP hook for it (SessionStart hooks are
    // command-type only), so thread.resume_session_id keeps pointing
    // at the cleared session until the new one's first prompt. A
    // daemon restart inside that window would relaunch with
    // `--resume <cleared>` and resurrect the session the user just
    // discarded. SessionEnd IS delivered over HTTP and carries the
    // ending session id + reason — drop the resume token when an
    // explicit clear ends exactly the session we'd resume.
    if event == "SessionEnd" {
        clear_resume_on_session_end(
            &ctx,
            thread_id.as_ref(),
            session_id.as_deref(),
            body_value.as_ref(),
        )
        .await;
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
        if let Some(deny) = pre_tool_check(&ctx, thread_id.as_ref(), body_value.as_ref()).await {
            // Persist the event with a deny outcome so the hook log
            // shows what the runtime did.
            let envelope = HookEnvelope {
                kind,
                thread_id,
                stream_id,
                session_id: session_id.clone(),
                payload_json: body_str,
                prompt: None,
            };
            let _ = ctx.services.hook_ingest.ingest(envelope).await;
            return (StatusCode::OK, Json(deny)).into_response();
        }
    }

    let envelope = HookEnvelope {
        kind,
        thread_id,
        stream_id,
        session_id,
        payload_json: body_str,
        prompt,
    };

    // Mine per-turn signals BEFORE ingest closes the open agent_turn
    // for Stop hooks. Cheap query (capped at 200 recent events) — only
    // runs for Stop, not on every hook.
    let turn_signals: Option<oxplow_app::agent_policy::TurnSignals> = if kind == HookKind::Stop {
        if let Some(tid) = thread_id.as_ref() {
            mine_turn_signals(&ctx, tid).await
        } else {
            None
        }
    } else {
        None
    };

    let envelope_for_resume = envelope.clone();
    if let Err(err) = ctx.services.hook_ingest.ingest(envelope).await {
        // The agent can't act on an error status — Claude Code just
        // prints a "non-blocking status code" warning into the user's
        // terminal. Log the cause server-side and ack anyway.
        warn!(?event, ?err, "hook ingest failed");
        return hook_ack();
    }

    // Resume-tracker: Claude Code drops HTTP hooks for SessionStart, so
    // we learn the session_id from whichever hook fires next. Persist
    // it onto the thread so the next agent spawn passes
    // `--resume <session_id>` and Claude actually picks up where it
    // left off (without this, every re-attach starts a fresh session).
    update_resume_session_id(&ctx, &envelope_for_resume).await;

    // Token usage (tsk104): on Stop, parse the transcript tail referenced
    // by the hook payload and record this turn's token delta against the
    // thread's open effort. Best-effort — never fail the hook on a parse
    // or IO error. See `.context/agent-model.md` (Token usage capture).
    if kind == HookKind::Stop {
        if let Some(thread_id) = envelope_for_resume.thread_id.as_ref() {
            if let Err(err) = ctx
                .services
                .token_usage
                .on_stop(
                    thread_id,
                    envelope_for_resume.session_id.as_deref(),
                    &envelope_for_resume.payload_json,
                )
                .await
            {
                warn!(?err, "token-usage capture failed");
            }
        }
    }

    // PostToolUse: attribute wiki-page edits to the originating thread
    // so the rail's "Finished" list can surface only the pages this
    // thread authored or revised.
    if kind == HookKind::PostToolUse {
        if let (Some(thread_id), Some(body)) =
            (envelope_for_resume.thread_id.as_ref(), body_value.as_ref())
        {
            attribute_wiki_page_edit(&ctx, thread_id, body).await;
            // Auto-claim structured edits onto the thread's open effort in
            // real time (claim-first attribution) — best-effort.
            attribute_effort_file_edit(&ctx, thread_id, body).await;
            // Persist the call (v_tool_call / v_context_read / v_struggle).
            record_tool_call(&ctx, thread_id, &envelope_for_resume.payload_json).await;

            // Collection: detect a test-run Bash command, record it
            // (observed), and ride along to coverage/analysis if configured.
            // Best-effort — never fail the hook on a collection error.
            //
            // DETACHED (tsk62): `bounded_hook_response` DROPS the handler
            // future at the 5s budget, and a test-run's recording can
            // legitimately outlive it (a debug-build junit ingest plus a
            // multi-MB lcov parse). Run inline, the coverage step after the
            // junit landed was silently cancelled on EVERY run — the 80%-target
            // coverage gauge never got a single fact. The work now runs on its
            // own task that always completes; the response waits briefly for
            // the nudge message so the fast path still steers the agent
            // inline. On timeout the nudge is still persisted by the task
            // (`persist_nudge`) — only the immediate injection is skipped.
            let services = ctx.services.clone();
            let collection_thread = *thread_id;
            let payload = envelope_for_resume.payload_json.clone();
            let (nudge_tx, nudge_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let nudge = match services
                    .collection
                    .on_post_tool_use(&collection_thread, &payload)
                    .await
                {
                    Ok(nudge) => nudge,
                    Err(err) => {
                        warn!(?err, "collection post-tool-use failed");
                        None
                    }
                };
                // Extension advisories (e.g. oxplow-analytics' coverage
                // target) ride the same additionalContext.
                let advisories = oxplow_app::advisories::for_thread(
                    &services,
                    &collection_thread,
                    oxplow_app::extensions::AdvisoryOn::PostToolUse,
                )
                .await;
                let combined: Vec<String> = nudge
                    .into_iter()
                    .chain(advisories.into_iter().map(|h| h.text))
                    .collect();
                let _ = nudge_tx.send((!combined.is_empty()).then(|| combined.join("\n\n")));
            });
            let collection_nudge = match tokio::time::timeout(
                std::time::Duration::from_millis(2500),
                nudge_rx,
            )
            .await
            {
                Ok(Ok(nudge)) => nudge,
                _ => None,
            };

            // ExitPlanMode just settled — if the thread was promoted
            // (or demoted) while sitting on the plan-mode approval
            // prompt, no UserPromptSubmit fires between the user
            // clicking "Leave plan mode" and the agent resuming. So
            // emit the ROLE CHANGE banner here directly via
            // hookSpecificOutput.additionalContext, which Claude
            // Code injects into the conversation as a system note.
            if body
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(|s| s == "ExitPlanMode")
                .unwrap_or(false)
            {
                if let Some(banner) = role_change_banner_for(
                    &ctx,
                    thread_id,
                    envelope_for_resume.session_id.as_deref(),
                )
                .await
                {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "hookSpecificOutput": {
                                "hookEventName": "PostToolUse",
                                "additionalContext": banner,
                            }
                        })),
                    )
                        .into_response();
                }
            }

            // A report-less test run was detected — surface the
            // collection nudge to the agent via additionalContext.
            // (ExitPlanMode is never a test-run Bash command, so this
            // never races the role-change banner above.)
            if let Some(nudge) = collection_nudge {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "hookSpecificOutput": {
                            "hookEventName": "PostToolUse",
                            "additionalContext": nudge,
                        }
                    })),
                )
                    .into_response();
            }
        }
    }

    // UserPromptSubmit: refresh the agent's view of stream + thread +
    // role when it changed. Captures the launch-time role on the first
    // prompt of each session so subsequent prompts can detect
    // promotions/demotions and append a loud ROLE CHANGE banner.
    if kind == HookKind::UserPromptSubmit {
        if let Some(thread_id) = envelope_for_resume.thread_id.as_ref() {
            // Independent context pieces ride this one additionalContext:
            // the session-context block (role/stream changes, deduped so it
            // only re-emits when it actually changes), extension advisories
            // for the open effort (e.g. oxplow-analytics' metric deltas), and
            // the effort's recorded decisions. Join whatever is present.
            let ctx_block = refreshed_session_context(
                &ctx,
                thread_id,
                envelope_for_resume.session_id.as_deref(),
            )
            .await;
            let advisory_hits = oxplow_app::advisories::for_thread(
                &ctx.services,
                thread_id,
                oxplow_app::extensions::AdvisoryOn::Prompt,
            )
            .await;
            let advisory_block = (!advisory_hits.is_empty()).then(|| {
                advisory_hits
                    .into_iter()
                    .map(|h| h.text)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            });
            let decisions_block = refreshed_decisions_context(
                &ctx,
                thread_id,
                envelope_for_resume.session_id.as_deref(),
            )
            .await;
            let combined: String = [ctx_block, advisory_block, decisions_block]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n");
            if !combined.is_empty() {
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
    // fires. We mine per-turn activity by scanning hook events
    // received since the open turn's started_at, BEFORE ingest
    // closes the turn. The signals fed in here:
    //   - turn_had_activity: any PreToolUse/PostToolUse fired
    //   - turn_had_writes: any Edit/Write/MultiEdit/NotebookEdit fired
    // Other signals (subagent-in-flight, turn_filed_ready_item)
    // need cross-tool correlation we haven't wired yet — defaulting
    // them to false is a soft-degrade that silences a few advisory
    // branches but doesn't emit wrong directives.
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

/// Run the shared agent policy (write guard, then filing) against the
/// PreToolUse payload and render a deny as Claude's `hookSpecificOutput`.
/// `None` allows. Tools neither rule can refuse skip the policy's I/O
/// (`claude_intent` returns `None` for them).
async fn pre_tool_check(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    body: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    use oxplow_runtime::policy::{PolicyDecision, ToolIntent};
    use oxplow_runtime::write_guard::{HookSpecificOutput, WriteGuardDeny};
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
        PolicyDecision::Deny { reason, .. } => serde_json::to_value(WriteGuardDeny {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PreToolUse",
                permission_decision: "deny",
                permission_decision_reason: reason,
            },
        })
        .ok(),
    }
}

/// When a PostToolUse hook reports an Edit/Write/MultiEdit/NotebookEdit
/// targeting a `.oxplow/wiki/<slug>.md` path, record an entry in the
/// per-thread wiki-page attribution table. Mirrors how main attributes
/// note touches via the runtime's PostToolUse handler. Tolerant of
/// missing fields — attribution is best-effort.
async fn attribute_wiki_page_edit(ctx: &AppCtx, thread_id: &ThreadId, body: &serde_json::Value) {
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return;
    }
    let tool_input = match body.get("tool_input") {
        Some(t) => t,
        None => return,
    };
    let raw_path = tool_input
        .get("file_path")
        .or_else(|| tool_input.get("notebook_path"))
        .or_else(|| tool_input.get("path"))
        .and_then(|v| v.as_str());
    let Some(path) = raw_path else { return };
    let Some(slug) = wiki_page_slug_from_path(path, &ctx.services.layout.project_dir) else {
        return;
    };
    if let Err(err) = ctx
        .services
        .wiki_page_thread_updates
        .touch(thread_id, &slug, oxplow_domain::Timestamp::now())
        .await
    {
        warn!(?err, slug, "wiki-page attribution failed");
    }
}

/// Auto-claim the file a structured edit tool just wrote onto the thread's
/// OPEN effort, in real time (Child 1 of the claim-first attribution epic).
/// Mirrors `attribute_wiki_page_edit`'s tool gating: only Edit / Write /
/// MultiEdit / NotebookEdit are auto-claimed — Bash / codegen / formatter
/// writes are intentionally excluded (they stay for snapshot
/// reconciliation). Best-effort: any failure is logged and skipped so the
/// hook never fails.
async fn attribute_effort_file_edit(ctx: &AppCtx, thread_id: &ThreadId, body: &serde_json::Value) {
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let project_dir = &ctx.services.layout.project_dir;
    let Some(rel) = effort_claim_path_from_edit(tool_name, body.get("tool_input"), project_dir)
    else {
        return;
    };
    if let Err(err) = ctx
        .services
        .tasks
        .claim_open_effort_file(
            &ctx.services.effort_store,
            thread_id,
            &rel,
            Some(project_dir),
        )
        .await
    {
        warn!(?err, path = rel, "effort file auto-claim failed");
    }
}

/// Persist a PostToolUse as an `agent_tool_call` row on the thread's open
/// effort. Best-effort: a failure is logged, never surfaced to the agent.
async fn record_tool_call(ctx: &AppCtx, thread_id: &ThreadId, payload_json: &str) {
    use oxplow_app::TaskEffortStore as _;
    let Some(parts) =
        oxplow_app::tool_calls::parse_tool_call(payload_json, &ctx.services.layout.project_dir)
    else {
        return;
    };
    let effort_id = match ctx
        .services
        .effort_store
        .find_open_for_thread(thread_id)
        .await
    {
        Ok(e) => e.map(|e| e.id.value()),
        Err(err) => {
            warn!(?err, "tool-call effort lookup failed");
            None
        }
    };
    let call = oxplow_db::NewToolCall {
        thread_id: thread_id.value(),
        effort_id,
        tool: parts.tool,
        path: parts.path,
        detail: parts.detail,
        ok: parts.ok,
    };
    if let Err(err) = ctx.services.tool_call_store.record(call).await {
        warn!(?err, "tool-call record failed");
    }
}

/// Repo-relative path to auto-claim from a structured edit tool, or `None`
/// when the tool isn't a structured write, no path is present, or the path
/// is an absolute path outside the project (not an effort file). Stored
/// `task_effort_file` paths are repo-relative, so an absolute path inside
/// the project is normalized against `project_dir`.
fn effort_claim_path_from_edit(
    tool_name: &str,
    tool_input: Option<&serde_json::Value>,
    project_dir: &Path,
) -> Option<String> {
    if !matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return None;
    }
    let raw = tool_input?
        .get("file_path")
        .or_else(|| tool_input?.get("notebook_path"))
        .or_else(|| tool_input?.get("path"))
        .and_then(|v| v.as_str())?;
    let path = Path::new(raw);
    if path.is_absolute() {
        // Absolute inside the project → repo-relative; outside → not an
        // effort file (strip_prefix fails → None).
        path.strip_prefix(project_dir)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
    } else {
        Some(raw.to_string())
    }
}

/// Map an Edit-tool file path to a wiki-page slug iff the path is
/// inside `.oxplow/wiki/` with a `.md` extension. Accepts absolute
/// or workspace-relative paths.
fn wiki_page_slug_from_path(raw: &str, project_dir: &Path) -> Option<String> {
    let path = Path::new(raw);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    };
    let notes_dir = project_dir.join(".oxplow").join("wiki");
    let rel = abs.strip_prefix(&notes_dir).ok()?;
    if rel
        .parent()
        .map(|p| !p.as_os_str().is_empty())
        .unwrap_or(false)
    {
        return None; // refuses subdirectories
    }
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    let ext = rel.extension()?.to_string_lossy();
    if ext != "md" {
        return None;
    }
    Some(stem)
}

/// Adopt the observed session_id as the thread's resume token when it
/// differs from the current value. Mirrors `decideResumeUpdate` from
/// `src/session/resume-tracker.ts`. Tolerant: any failure is logged
/// and skipped — resume tracking is best-effort.
/// Pure dedup decision: skip the resume tracker's DB work when the
/// in-memory cache already records this exact session id as persisted
/// for the thread. An empty / mismatched / absent cache entry means we
/// must hit the store to be sure.
fn resume_cache_allows_skip(cached: Option<&str>, observed: &str) -> bool {
    cached == Some(observed)
}

async fn update_resume_session_id(ctx: &AppCtx, env: &HookEnvelope) {
    let Some(observed) = env.session_id.as_deref() else {
        return;
    };
    if observed.is_empty() {
        return;
    }
    let Some(thread_id) = env.thread_id.as_ref() else {
        return;
    };
    // Fast path: the resume id only changes once per session, so once a
    // thread's id is cached every later hook short-circuits before the
    // DB. (The cache mirrors what we last persisted; a stale entry only
    // ever causes one redundant read, never a wrong write.)
    {
        let cache = ctx.resume_state.lock();
        if resume_cache_allows_skip(cache.get(thread_id).map(|s| s.as_str()), observed) {
            return;
        }
    }
    let thread = match ctx.services.thread_store.get(thread_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(err) => {
            warn!(?err, "resume-tracker: thread lookup failed");
            return;
        }
    };
    if thread.resume_session_id == observed {
        // DB already in sync — seed the cache so the next hook skips
        // this read (cold cache after restart hits this branch once).
        ctx.resume_state
            .lock()
            .insert(*thread_id, observed.to_string());
        return;
    }
    let mut updated = thread;
    updated.resume_session_id = observed.to_string();
    updated.updated_at = oxplow_domain::Timestamp::now();
    if let Err(err) = ctx.services.thread_store.upsert(&updated).await {
        warn!(?err, "resume-tracker: thread upsert failed");
        return;
    }
    // Record what we just persisted so repeat hooks short-circuit.
    ctx.resume_state
        .lock()
        .insert(*thread_id, observed.to_string());
}

/// Pure decision for the SessionEnd branch: drop the thread's resume
/// token only when an explicit `/clear` ended exactly the session the
/// token points at. Normal exits (`other`, `prompt_input_exit`,
/// `logout`) keep the token so a restart resumes the conversation, and
/// a clear of a stale session must not wipe a newer token.
fn resume_should_clear(reason: Option<&str>, ended_session: &str, current_resume: &str) -> bool {
    reason == Some("clear") && !ended_session.is_empty() && ended_session == current_resume
}

/// Apply [`resume_should_clear`] against the thread row. Tolerant like
/// the resume tracker — failures are logged and skipped.
async fn clear_resume_on_session_end(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    session_id: Option<&str>,
    body: Option<&serde_json::Value>,
) {
    let (Some(thread_id), Some(ended)) = (thread_id, session_id) else {
        return;
    };
    let reason = body.and_then(|v| v.get("reason")).and_then(|r| r.as_str());
    let thread = match ctx.services.thread_store.get(thread_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(err) => {
            warn!(?err, "resume-tracker: thread lookup failed on SessionEnd");
            return;
        }
    };
    if !resume_should_clear(reason, ended, &thread.resume_session_id) {
        return;
    }
    let mut updated = thread;
    updated.resume_session_id = String::new();
    updated.updated_at = oxplow_domain::Timestamp::now();
    if let Err(err) = ctx.services.thread_store.upsert(&updated).await {
        warn!(?err, "resume-tracker: clearing resume token failed");
    }
}

async fn mine_turn_signals(
    ctx: &AppCtx,
    thread_id: &ThreadId,
) -> Option<oxplow_app::agent_policy::TurnSignals> {
    let open = ctx
        .services
        .agent_turn_store
        .list_open(thread_id)
        .await
        .ok()?;
    let started_at = open.first()?.started_at;
    let events = ctx
        .services
        .hook_event_store
        .list_recent(Some(thread_id), 200)
        .await
        .ok()?;
    let mut signals = oxplow_app::agent_policy::TurnSignals::default();
    for evt in events {
        if evt.received_at < started_at {
            continue;
        }
        if !matches!(evt.kind, HookKind::PreToolUse | HookKind::PostToolUse) {
            continue;
        }
        signals.had_activity = true;
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&evt.payload_json) {
            if let Some(tool_name) = payload.get("tool_name").and_then(|v| v.as_str()) {
                if matches!(tool_name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                    signals.had_writes = true;
                }
            }
        }
    }
    Some(signals)
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

/// Look up (or capture, if first time we see this session) the role
/// this thread was launched with for the given Claude session_id,
/// then return it. None when no session_id was supplied (the agent
/// hasn't reported one yet via any hook).
async fn capture_or_get_initial_role(
    ctx: &AppCtx,
    thread_id: &ThreadId,
    session_id: Option<&str>,
) -> Option<RoleMode> {
    let session_id = session_id?.to_string();
    let thread = ctx
        .services
        .thread_store
        .get(thread_id)
        .await
        .ok()
        .flatten()?;
    let current = RoleMode::from_thread(&thread);
    let mut st = ctx.role_state.lock();
    Some(
        *st.initial_role_by_session_id
            .entry(session_id)
            .or_insert(current),
    )
}

/// Build a fresh `<session-context>` block for the thread with the
/// initial-role banner attached when the role has flipped. Returns
/// None when stream/thread lookups fail or the project disables
/// session-context injection. Caller wraps the returned string in
/// `hookSpecificOutput.additionalContext`.
async fn refreshed_session_context(
    ctx: &AppCtx,
    thread_id: &ThreadId,
    session_id: Option<&str>,
) -> Option<String> {
    let cfg = ctx.services.config.read().ok()?.clone();
    if !cfg.inject_session_context {
        return None;
    }
    let thread = ctx
        .services
        .thread_store
        .get(thread_id)
        .await
        .ok()
        .flatten()?;
    let stream = ctx
        .services
        .stream_store
        .get(&thread.stream_id)
        .await
        .ok()
        .flatten()?;
    let initial = capture_or_get_initial_role(ctx, thread_id, session_id).await;
    let block = build_session_context_block_with_role(&stream, Some(&thread), initial);
    should_emit_session_context(&ctx.role_state, session_id, &block).then_some(block)
}

/// The open effort's recorded decisions as context, emitted on the first
/// prompt of a session (so they survive a compaction / resume, which
/// resets the baseline via `SessionStart`) and again whenever they change.
async fn refreshed_decisions_context(
    ctx: &AppCtx,
    thread_id: &ThreadId,
    session_id: Option<&str>,
) -> Option<String> {
    use oxplow_app::TaskEffortStore as _;
    let effort = ctx
        .services
        .effort_store
        .find_open_for_thread(thread_id)
        .await
        .ok()
        .flatten()?;
    let block = oxplow_app::reasoning::effort_decisions_block(
        &oxplow_db::SemanticLayer::new(ctx.services.db.clone()),
        effort.id.value(),
    )
    .await?;
    let key = session_id.map(|s| format!("{s}{DECISIONS_KEY_SUFFIX}"));
    should_emit_session_context(&ctx.role_state, key.as_deref(), &block).then_some(block)
}

/// Dedupe key suffix for the decisions block, beside the session-context
/// block's plain session-id key.
const DECISIONS_KEY_SUFFIX: &str = "#decisions";

fn should_emit_session_context(
    state: &Mutex<RoleState>,
    session_id: Option<&str>,
    block: &str,
) -> bool {
    let Some(session_id) = session_id else {
        // Without a stable identity, suppressing could hide a context
        // change from a different session that happens to share a
        // thread. Prefer the small duplicate over stale instructions.
        return true;
    };
    let mut state = state.lock();
    match state.last_context_by_session_id.get(session_id) {
        Some(previous) if previous == block => false,
        _ => {
            state
                .last_context_by_session_id
                .insert(session_id.to_string(), block.to_string());
            true
        }
    }
}

fn reset_session_context_state(state: &Mutex<RoleState>, session_id: Option<&str>) {
    let Some(session_id) = session_id else {
        return;
    };
    let mut state = state.lock();
    state.initial_role_by_session_id.remove(session_id);
    state.last_context_by_session_id.remove(session_id);
    state
        .last_context_by_session_id
        .remove(&format!("{session_id}{DECISIONS_KEY_SUFFIX}"));
}

/// Returns just the ROLE CHANGE sentence (no surrounding session-
/// context block) when the thread's current role differs from the
/// initial role recorded for this session. None when there's no
/// captured baseline yet, the lookup fails, or the role hasn't
/// changed. Used by the ExitPlanMode PostToolUse path which only
/// needs the banner — the agent already has a fresh session-context
/// from the most recent UserPromptSubmit.
async fn role_change_banner_for(
    ctx: &AppCtx,
    thread_id: &ThreadId,
    session_id: Option<&str>,
) -> Option<String> {
    let session_id = session_id?.to_string();
    let thread = ctx
        .services
        .thread_store
        .get(thread_id)
        .await
        .ok()
        .flatten()?;
    let current = RoleMode::from_thread(&thread);
    let initial = {
        let st = ctx.role_state.lock();
        st.initial_role_by_session_id.get(&session_id).copied()
    }?;
    if initial == current {
        return None;
    }
    Some(role_change_banner(initial, current))
}

fn parse_hook_kind(event: &str) -> Option<HookKind> {
    match event {
        "PreToolUse" => Some(HookKind::PreToolUse),
        "PostToolUse" => Some(HookKind::PostToolUse),
        "UserPromptSubmit" => Some(HookKind::UserPromptSubmit),
        "Stop" => Some(HookKind::Stop),
        // SessionStart / SessionEnd / Notification aren't on the
        // HookKind enum yet — they're informational from oxplow's
        // perspective. Returning None routes them to the 200 ack above
        // without persisting. AgentBoot, SubagentStop, Interrupt are
        // synthetic / not posted by the plugin.
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
    fn resume_clear_decision() {
        // Only an explicit clear of the exact resume session drops it.
        assert!(resume_should_clear(Some("clear"), "s1", "s1"));
        // Other exit reasons keep the token (restart should resume).
        assert!(!resume_should_clear(Some("other"), "s1", "s1"));
        assert!(!resume_should_clear(Some("prompt_input_exit"), "s1", "s1"));
        assert!(!resume_should_clear(None, "s1", "s1"));
        // A clear of a stale session must not wipe a newer token.
        assert!(!resume_should_clear(Some("clear"), "old", "newer"));
        // Degenerate ids never match.
        assert!(!resume_should_clear(Some("clear"), "", ""));
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
    fn wiki_slug_from_relative_path_in_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A relative path that resolves into .oxplow/wiki returns the slug.
        let slug = wiki_page_slug_from_path(".oxplow/wiki/architecture.md", tmp.path());
        assert_eq!(slug.as_deref(), Some("architecture"));
    }

    #[test]
    fn wiki_slug_from_absolute_path_in_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let abs = tmp.path().join(".oxplow/wiki/data-model.md");
        let slug = wiki_page_slug_from_path(&abs.to_string_lossy(), tmp.path());
        assert_eq!(slug.as_deref(), Some("data-model"));
    }

    #[test]
    fn wiki_slug_rejects_non_md_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path(".oxplow/wiki/foo.txt", tmp.path()).is_none());
        // No extension at all.
        assert!(wiki_page_slug_from_path(".oxplow/wiki/foo", tmp.path()).is_none());
    }

    #[test]
    fn wiki_slug_rejects_subdirectory_paths() {
        // Wiki notes must be flat under .oxplow/wiki — a path with a
        // subdirectory shouldn't accidentally adopt the basename.
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path(".oxplow/wiki/sub/inner.md", tmp.path()).is_none());
    }

    #[test]
    fn wiki_slug_rejects_paths_outside_notes_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(wiki_page_slug_from_path("README.md", tmp.path()).is_none());
        assert!(wiki_page_slug_from_path(".oxplow/other/foo.md", tmp.path()).is_none());
        assert!(wiki_page_slug_from_path("/etc/hosts", tmp.path()).is_none());
    }

    #[test]
    fn effort_claim_path_extracts_repo_relative_for_structured_tools() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Relative file_path → returned as-is (already repo-relative).
        let ti = serde_json::json!({ "file_path": "src/edited.rs" });
        assert_eq!(
            effort_claim_path_from_edit("Edit", Some(&ti), tmp.path()).as_deref(),
            Some("src/edited.rs")
        );
        // Absolute path inside the project → normalized to repo-relative.
        let abs = tmp.path().join("crates/x/lib.rs");
        let ti_abs = serde_json::json!({ "file_path": abs.to_string_lossy() });
        assert_eq!(
            effort_claim_path_from_edit("Write", Some(&ti_abs), tmp.path()).as_deref(),
            Some("crates/x/lib.rs")
        );
        // NotebookEdit uses notebook_path.
        let ti_nb = serde_json::json!({ "notebook_path": "nb/run.ipynb" });
        assert_eq!(
            effort_claim_path_from_edit("NotebookEdit", Some(&ti_nb), tmp.path()).as_deref(),
            Some("nb/run.ipynb")
        );
    }

    #[test]
    fn effort_claim_path_excludes_bash_and_outside_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Bash (and any non-structured tool) is intentionally NOT auto-claimed.
        let ti = serde_json::json!({ "command": "echo hi > out.txt" });
        assert!(effort_claim_path_from_edit("Bash", Some(&ti), tmp.path()).is_none());
        // An absolute path outside the project is not an effort file.
        let ti_out = serde_json::json!({ "file_path": "/etc/hosts" });
        assert!(effort_claim_path_from_edit("Edit", Some(&ti_out), tmp.path()).is_none());
        // Missing path → None.
        let ti_empty = serde_json::json!({});
        assert!(effort_claim_path_from_edit("Edit", Some(&ti_empty), tmp.path()).is_none());
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

    #[test]
    fn resume_cache_skips_only_on_exact_match() {
        // Cache hit: the thread already has this session id persisted →
        // skip the DB round-trip entirely.
        assert!(resume_cache_allows_skip(Some("s1"), "s1"));
        // Cache miss / changed / first-seen → must hit the DB.
        assert!(!resume_cache_allows_skip(None, "s1"));
        assert!(!resume_cache_allows_skip(Some("s0"), "s1"));
        // Degenerate empty cached value never matches a real id.
        assert!(!resume_cache_allows_skip(Some(""), "s1"));
    }

    #[test]
    fn session_context_emits_initial_and_changed_blocks_only() {
        let state = Mutex::new(RoleState::default());

        assert!(should_emit_session_context(
            &state,
            Some("session-1"),
            "context-a"
        ));
        assert!(!should_emit_session_context(
            &state,
            Some("session-1"),
            "context-a"
        ));
        assert!(should_emit_session_context(
            &state,
            Some("session-1"),
            "context-b"
        ));
    }

    #[test]
    fn session_context_without_session_id_is_never_suppressed() {
        let state = Mutex::new(RoleState::default());
        assert!(should_emit_session_context(&state, None, "context"));
        assert!(should_emit_session_context(&state, None, "context"));
    }

    #[test]
    fn clearing_session_context_baseline_allows_fresh_emission() {
        let state = Mutex::new(RoleState::default());
        state
            .lock()
            .initial_role_by_session_id
            .insert("session-1".into(), RoleMode::Writer);
        assert!(should_emit_session_context(
            &state,
            Some("session-1"),
            "context"
        ));
        reset_session_context_state(&state, Some("session-1"));
        assert!(!state
            .lock()
            .initial_role_by_session_id
            .contains_key("session-1"));
        assert!(should_emit_session_context(
            &state,
            Some("session-1"),
            "context"
        ));
    }
}
