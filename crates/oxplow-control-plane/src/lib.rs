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
use rand::RngCore;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use oxplow_app::{
    build_session_context_block_with_role, role_change_banner, HookEnvelope, RoleMode, Services,
};
use oxplow_domain::stores::{AgentTurnStore, StreamStore, TaskStore, ThreadStore};
use oxplow_domain::{HookKind, StreamId, TaskStatus, Thread, ThreadId};
use oxplow_runtime::filing::{build_filing_enforcement_pre_tool_deny, FilingEnforcementContext};
use oxplow_runtime::stop_hook::{
    decide_stop_directive, DirectiveBuilders, PendingEffortReview, StopHookSideEffect,
    ThreadSnapshot,
};
use oxplow_runtime::write_guard::{build_write_guard_response, WriteGuardContext};

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
}

/// In-memory state the Stop pipeline needs across hook events. Lives
/// here (not in `Services`) because main treated it as runtime-only
/// state — losing it on a daemon restart is acceptable: the worst
/// case is one duplicate audit nudge after restart.
#[derive(Default)]
struct StopState {
    /// Last in-progress audit signature emitted per thread; used to
    /// dedupe back-to-back audits when the in_progress set hasn't
    /// changed.
    last_audit_signature: HashMap<ThreadId, String>,
    /// Threads where the runtime has already fired the
    /// "filed-but-didn't-ship" advisory this turn.
    filed_but_didnt_ship_fired: HashMap<ThreadId, bool>,
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
    stop_state: Arc<Mutex<StopState>>,
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
        stop_state: Arc::new(Mutex::new(StopState::default())),
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
    rand::rng().fill_bytes(&mut bytes);
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
    let turn_signals: Option<TurnSignals> = if kind == HookKind::Stop {
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

            // Collection: detect a test-run Bash command, record it
            // (observed), and ride along to coverage if configured.
            // Best-effort — never fail the hook on a collection error.
            let collection_nudge = match ctx
                .services
                .collection
                .on_post_tool_use(thread_id, &envelope_for_resume.payload_json)
                .await
            {
                Ok(nudge) => nudge,
                Err(err) => {
                    warn!(?err, "collection post-tool-use failed");
                    None
                }
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
            // Two independent context pieces ride this one additionalContext:
            // the session-context block (role/stream changes, deduped so it
            // only re-emits when it actually changes) and the advisory metric
            // deltas for the open effort (tsk231, recomputed each turn). Join
            // whatever is present.
            let ctx_block = refreshed_session_context(
                &ctx,
                thread_id,
                envelope_for_resume.session_id.as_deref(),
            )
            .await;
            let metric_block = ctx
                .services
                .collection
                .effort_metric_context(thread_id)
                .await;
            let combined: String = [ctx_block, metric_block]
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

/// Whether `pre_tool_check` could possibly produce a deny for this tool.
/// Both guards bail to `None` for any tool outside the worktree-mutating
/// set: write_guard checks `WORKTREE_MUTATING_TOOLS`, filing checks
/// `ALWAYS_WRITE_INTENT_TOOL_NAMES` — identical sets. So for everything
/// else (Read / Grep / Bash / mcp / Task / WebFetch / …) the full check
/// is provably a no-op, and the runtime can skip the thread + task-list
/// DB reads and the git-state stat syscalls entirely. Kept as a pure fn
/// so the equivalence is unit-testable against the canonical lists.
fn pre_tool_check_applies(tool_name: &str) -> bool {
    use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
    use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
    WORKTREE_MUTATING_TOOLS.contains(&tool_name)
        || ALWAYS_WRITE_INTENT_TOOL_NAMES.contains(&tool_name)
}

/// Run write_guard then filing_enforcement against the PreToolUse
/// payload. Returns the first deny body that fires, or None to allow.
async fn pre_tool_check(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    body: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let thread_id = thread_id?;
    let body = body?;
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    // Fast path: neither guard can ever deny a tool that isn't a
    // worktree-mutating edit, so skip the DB reads + git-state stats for
    // the common case (Read / Grep / Bash / mcp / Task / …). Persistence
    // is unaffected — `handle_hook_inner` ingests the event regardless of
    // what this returns. See `pre_tool_check_applies`.
    if !pre_tool_check_applies(tool_name) {
        return None;
    }
    let tool_input = body.get("tool_input");

    let thread = ctx
        .services
        .thread_store
        .get(thread_id)
        .await
        .ok()
        .flatten()?;

    let project_dir = ctx.services.layout.project_dir.as_path();

    let file_path = tool_input
        .and_then(|t| {
            t.get("file_path")
                .or_else(|| t.get("notebook_path"))
                .or_else(|| t.get("path"))
        })
        .and_then(|v| v.as_str());

    // Out-of-worktree edits are none of oxplow's concern: an absolute path
    // outside the project root can't be a project file, can't be claimed by an
    // effort, and can't be in any effort's changed set. Allow cleanly BEFORE
    // either guard so editing e.g. the Claude Code plan file under `~/.claude/`
    // never trips the write-guard (read-only thread) or filing enforcement, and
    // never surfaces a non-blocking hook error (tsk212). (PostToolUse's
    // auto-claim already no-ops out-of-worktree via `effort_claim_path_from_edit`.)
    if edit_path_outside_worktree(file_path, project_dir) {
        return None;
    }

    // Layer 1: write_guard for read-only threads.
    if let Some(deny) = build_write_guard_response(
        Some(&thread),
        tool_name,
        WriteGuardContext {
            project_dir: Some(project_dir),
            tool_input,
        },
    ) {
        return serde_json::to_value(deny).ok();
    }

    // Layer 2: filing_enforcement for the writer thread.
    let has_in_progress_task = stream_has_in_progress_claim(ctx, &thread).await;

    let git_operation_in_progress = git_operation_in_progress(project_dir);

    if let Some(deny) = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
        thread: Some(&thread),
        tool_name,
        has_in_progress_task,
        file_path,
        git_operation_in_progress,
    }) {
        return serde_json::to_value(deny).ok();
    }

    None
}

/// Whether the stream's active writer has a claimed (`in_progress`) task
/// that satisfies filing enforcement.
///
/// Scoped to the whole STREAM, not just the literal thread the task was
/// filed on (tsk133). A stream has exactly one active writer (enforced by
/// the `idx_threads_one_active_per_stream` unique index + the write
/// guard), so any `in_progress` task on *any* thread in that stream is a
/// legitimate claim for the writer. This is what makes cross-thread
/// dispatch work: a task filed on a sibling thread and routed to the
/// stream's writer no longer needs a manual `move_task` first. The core
/// invariant is untouched — queued/closed threads still can't write
/// (the write guard runs first); only which thread's `in_progress` row
/// counts as the writer's claim changes.
async fn stream_has_in_progress_claim(ctx: &AppCtx, thread: &Thread) -> bool {
    let threads = match ctx
        .services
        .thread_store
        .list_for_stream(&thread.stream_id)
        .await
    {
        Ok(threads) => threads,
        // On a lookup failure, fall back to the literal thread so the
        // guard still works for the common (same-thread) case.
        Err(_) => vec![thread.clone()],
    };
    for t in &threads {
        let claimed = ctx
            .services
            .task_store
            .list_for_thread(&t.id)
            .await
            .map(|items| items.iter().any(|i| i.status == TaskStatus::InProgress))
            .unwrap_or(false);
        if claimed {
            return true;
        }
    }
    false
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

/// True when a structured-edit `file_path` is an **absolute path outside** the
/// project worktree — none of oxplow's concern, so both PreToolUse guards
/// short-circuit to a clean allow (tsk212). Relative paths (resolved against
/// the worktree) and `None` fall through to the normal guards.
fn edit_path_outside_worktree(file_path: Option<&str>, project_dir: &Path) -> bool {
    match file_path {
        Some(p) => {
            let path = Path::new(p);
            path.is_absolute() && !path.starts_with(project_dir)
        }
        None => false,
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

/// Returns true when the worktree is mid git merge / rebase /
/// cherry-pick / revert. Filing enforcement exempts edits in these
/// states because conflict resolution can't dead-lock against the
/// filing rule. Mirrors `src/electron/filing-enforcement.ts`.
fn git_operation_in_progress(project_dir: &Path) -> bool {
    let gitdir = project_dir.join(".git");
    for marker in [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        if gitdir.join(marker).exists() {
            return true;
        }
    }
    // Worktrees: .git is a file pointing at the real gitdir.
    if let Ok(contents) = std::fs::read_to_string(&gitdir) {
        if let Some(real_dir) = contents.strip_prefix("gitdir: ") {
            let real = Path::new(real_dir.trim());
            for marker in [
                "MERGE_HEAD",
                "REBASE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
            ] {
                if real.join(marker).exists() {
                    return true;
                }
            }
        }
    }
    false
}

/// Per-turn signals reconstructed from the hook_event_store between
/// the open agent_turn's started_at and now. Powers the Stop
/// pipeline's Q&A-turn carve-out and the writes-vs-no-writes branch
/// of the filed-but-didn't-ship advisory.
#[derive(Debug, Clone, Default)]
struct TurnSignals {
    /// At least one PreToolUse / PostToolUse fired since the turn opened.
    had_activity: bool,
    /// At least one Edit/Write/MultiEdit/NotebookEdit fired since the turn opened.
    had_writes: bool,
}

async fn mine_turn_signals(ctx: &AppCtx, thread_id: &ThreadId) -> Option<TurnSignals> {
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
    let mut signals = TurnSignals::default();
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

/// Build a Stop directive for the writer thread. Pulls the current
/// in_progress set, runs `decide_stop_directive` with the in-memory
/// audit-signature dedup, and persists the side effects back to
/// `StopState`.
async fn stop_directive(
    ctx: &AppCtx,
    thread_id: Option<&ThreadId>,
    turn_signals: Option<&TurnSignals>,
) -> Option<serde_json::Value> {
    use oxplow_db::TaskEffortStore as _;
    let thread_id = thread_id?;
    let thread = ctx
        .services
        .thread_store
        .get(thread_id)
        .await
        .ok()
        .flatten()?;

    let tasks = ctx
        .services
        .task_store
        .list_for_thread(thread_id)
        .await
        .ok()
        .unwrap_or_default();

    let last_signature = ctx
        .stop_state
        .lock()
        .last_audit_signature
        .get(thread_id)
        .cloned();
    let filed_but_didnt_ship_fired = ctx
        .stop_state
        .lock()
        .filed_but_didnt_ship_fired
        .get(thread_id)
        .copied()
        .unwrap_or(false);

    // Drain any pending effort review ids the MCP `complete_task`
    // handler stashed for this thread. For each, recompute the
    // review against the live `task_effort_file` rows so an agent
    // that already amended doesn't get a stale prompt. Drop the ones
    // that no longer carry a discrepancy. Title resolution joins
    // each effort's task title for the directive text.
    let pending_ids = ctx
        .services
        .thread_runtime
        .take_pending_effort_reviews(thread_id);
    let mut pending_reviews: Vec<PendingEffortReview> = Vec::new();
    if !pending_ids.is_empty() {
        let titles_by_id: std::collections::HashMap<i64, String> = tasks
            .iter()
            .map(|t| (t.id.value(), t.title.clone()))
            .collect();
        for eid in pending_ids {
            // Two reconcilable kinds share this surface: files (recomputed
            // against live `task_effort_file` rows) and test runs (the
            // `effort_attribution` ledger's unattributed residue). An effort
            // is worth surfacing if EITHER still carries something to triage.
            let file_review = oxplow_app::task_service::recompute_effort_file_review(
                &ctx.services.effort_store,
                &ctx.services.snapshot_store,
                &eid,
            )
            .await;
            let unattributed_refs = ctx
                .services
                .attribution_store
                .list_refs(&eid, "run", oxplow_db::STATE_UNATTRIBUTED)
                .await
                .unwrap_or_default();
            if file_review.is_none() && unattributed_refs.is_empty() {
                continue;
            }
            // Enrich each bare `run:<id>` ref into a human descriptor the agent
            // can actually recognize ("run:47 — cargo test (419 passed, 0
            // failed) @ 10:03") — the ref stays at the front so `claim_runs`
            // still parses it (tsk266).
            let mut unattributed_runs = Vec::with_capacity(unattributed_refs.len());
            for r in &unattributed_refs {
                unattributed_runs.push(describe_run(&ctx.services.metric_store, r).await);
            }
            // task_id/title come from the file review when present; otherwise
            // resolve from the effort (run-only residue, no file discrepancy).
            let task_id = match file_review.as_ref() {
                Some(r) => r.task_id,
                None => ctx
                    .services
                    .effort_store
                    .get_effort(&eid)
                    .await
                    .ok()
                    .flatten()
                    .map(|e| e.task_id.value())
                    .unwrap_or(0),
            };
            let title = titles_by_id
                .get(&task_id)
                .cloned()
                .unwrap_or_else(|| format!("task {task_id}"));
            pending_reviews.push(PendingEffortReview {
                effort_id: file_review
                    .as_ref()
                    .map(|r| r.effort_id.clone())
                    .unwrap_or_else(|| eid.value().to_string()),
                task_id,
                task_title: title,
                claimed_but_not_changed: file_review
                    .as_ref()
                    .map(|r| r.claimed_but_not_changed.clone())
                    .unwrap_or_default(),
                changed_but_not_claimed: file_review
                    .as_ref()
                    .map(|r| r.changed_but_not_claimed.clone())
                    .unwrap_or_default(),
                unclaimed_overflow: file_review.as_ref().and_then(|r| r.unclaimed_overflow),
                unattributed_runs,
            });
        }
    }

    let snapshot = ThreadSnapshot {
        thread: Some(&thread),
        tasks: &tasks,
        last_in_progress_audit_signature: last_signature.as_deref(),
        // Mined from hook_event_store between this turn's started_at
        // and now. Letting the Q&A-turn carve-out fire silences the
        // audit nudge on read-only / one-off question turns where
        // there's no work to claim.
        turn_had_activity: turn_signals.map(|s| s.had_activity),
        turn_had_writes: turn_signals.map(|s| s.had_writes).unwrap_or(false),
        // Not yet wired (default false ⇒ branches stay silent rather
        // than emit wrong directives):
        // - subagent_in_flight: would need PreToolUse(Task) /
        //   SubagentStop correlation
        // - turn_had_filing / turn_filed_ready_item: would need MCP
        //   call attribution back to this thread/turn
        // - awaiting_user: only set when await_user MCP tool fires,
        //   which is tracked via agent_status_store but not surfaced
        //   here yet
        subagent_in_flight: false,
        awaiting_user: false,
        turn_had_filing: false,
        turn_filed_ready_item: false,
        filed_but_didnt_ship_fired,
        pending_effort_reviews: &pending_reviews,
    };

    let outcome = decide_stop_directive(
        snapshot,
        DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_in_progress_audit_reason),
            build_filed_but_didnt_ship_reason: Some(&build_filed_but_didnt_ship_reason),
            build_stale_epic_children_reason: None,
            build_effort_file_review_reason: Some(&build_effort_file_review_reason),
        },
    );

    // Apply side effects to the in-memory state.
    {
        let mut st = ctx.stop_state.lock();
        for eff in &outcome.side_effects {
            match eff {
                StopHookSideEffect::RecordAuditSignature(sig) => {
                    st.last_audit_signature.insert(*thread_id, sig.clone());
                }
                StopHookSideEffect::RecordFiledButDidntShipFired => {
                    st.filed_but_didnt_ship_fired.insert(*thread_id, true);
                }
            }
        }
    }

    outcome.directive.and_then(|d| serde_json::to_value(d).ok())
}

fn build_in_progress_audit_reason(items: &[oxplow_domain::Task]) -> String {
    let titles: Vec<String> = items
        .iter()
        .map(|i| format!("  • [{}] {}", i.id.value(), i.title))
        .collect();
    format!(
        "AUDIT: this turn is closing with {} task(s) still `in_progress`:\n{}\n\n\
         Before stopping, walk each one:\n\
         - Done? → `mcp__oxplow__complete_task` with `touchedFiles`.\n\
         - Stale or no longer the right shape? → `mcp__oxplow__update_task` to ready/blocked/done.\n\
         - Waiting on the user? → `mcp__oxplow__await_user`.\n\n\
         An `in_progress` row with finished work parked in it looks stuck to the user.",
        items.len(),
        titles.join("\n")
    )
}

/// Compose the human descriptor for a run from its already-fetched parts. Pure
/// so it's unit-testable; [`describe_run`] does the I/O and calls this (tsk266).
/// The `run:<id>` ref leads so `claim_runs`/`disclaim_runs` can still parse it.
fn format_run_descriptor(
    run_ref: &str,
    summary: Option<&str>,
    time_hm: Option<(u8, u8)>,
) -> String {
    let mut out = run_ref.to_string();
    if let Some(s) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!(" — {s}"));
    }
    if let Some((h, m)) = time_hm {
        out.push_str(&format!(" @ {h:02}:{m:02}"));
    }
    out
}

/// Render the kind-specific middle of a run descriptor from its `*-detail`
/// finding payload — tests show command + pass/fail, coverage shows the diff %,
/// analysis shows the analyzer + error/warning counts (tsk269). Pure for testing.
fn run_summary_from_detail(detail_kind: &str, payload: &serde_json::Value) -> Option<String> {
    let counts = |parts: &[(&str, &str)]| -> String {
        let joined: Vec<String> = parts
            .iter()
            .filter_map(|(key, label)| {
                payload
                    .get(*key)
                    .and_then(serde_json::Value::as_i64)
                    .map(|n| format!("{n} {label}"))
            })
            .collect();
        if joined.is_empty() {
            String::new()
        } else {
            format!(" ({})", joined.join(", "))
        }
    };
    let str_field = |key: &str| payload.get(key).and_then(|v| v.as_str());
    match detail_kind {
        "test-detail" => {
            let cmd = str_field("command").unwrap_or("test run").trim();
            Some(format!(
                "{cmd}{}",
                counts(&[("passed", "passed"), ("failed", "failed")])
            ))
        }
        "coverage-detail" => Some(
            match payload
                .get("summaryPct")
                .and_then(serde_json::Value::as_f64)
            {
                Some(pct) => format!("coverage ({pct:.0}% of changed lines)"),
                None => "coverage report".to_string(),
            },
        ),
        "analysis-detail" => {
            let who = str_field("analyzer")
                .or_else(|| str_field("command"))
                .unwrap_or("analysis")
                .trim();
            Some(format!(
                "{who}{}",
                counts(&[("errorCount", "errors"), ("warningCount", "warnings")])
            ))
        }
        _ => None,
    }
}

/// Turn a bare `run:<id>` ledger ref into a descriptor by joining the
/// `metric_run` row (timestamp) + its `*-detail` finding (test/coverage/analysis)
/// from the substrate. Dispatches on the finding kind so a coverage/analysis run
/// renders as such, not a malformed test run (tsk266/tsk269). Falls back to the
/// bare ref when the run can't be looked up — never blocks the review.
async fn describe_run(metrics: &oxplow_db::SqliteMetricStore, run_ref: &str) -> String {
    let Some(id) = run_ref
        .strip_prefix("run:")
        .and_then(|s| s.parse::<i64>().ok())
    else {
        return run_ref.to_string();
    };
    let summary = metrics
        .list_findings(id)
        .await
        .ok()
        .unwrap_or_default()
        .into_iter()
        .find_map(|f| {
            let payload =
                serde_json::from_str::<serde_json::Value>(f.extra_json.as_deref()?).ok()?;
            run_summary_from_detail(&f.kind, &payload)
        });
    let time_hm = metrics
        .get_run(id)
        .await
        .ok()
        .flatten()
        .map(|r| (r.started_at.0.hour(), r.started_at.0.minute()));
    format_run_descriptor(run_ref, summary.as_deref(), time_hm)
}

fn build_effort_file_review_reason(reviews: &[PendingEffortReview]) -> String {
    let mut out = String::from(
        "EFFORT REVIEW: one or more efforts you just closed have a discrepancy between \
         what you declared and what oxplow observed — in the files you touched and/or \
         the test/coverage/analysis runs that happened during your effort. For each:\n\n",
    );
    for r in reviews {
        out.push_str(&format!(
            "  • [{}] {} (effort {})\n",
            r.task_id, r.task_title, r.effort_id
        ));
        if !r.claimed_but_not_changed.is_empty() {
            out.push_str("      You claimed these files but the worktree didn't change:\n");
            for p in &r.claimed_but_not_changed {
                out.push_str(&format!("        - {p}\n"));
            }
        }
        if !r.changed_but_not_claimed.is_empty() {
            out.push_str(
                "      These files changed during your effort but you didn't list them:\n",
            );
            for p in &r.changed_but_not_claimed {
                out.push_str(&format!("        - {p}\n"));
            }
        }
        if let Some(total) = r.unclaimed_overflow {
            out.push_str(&format!(
                "      ({total} files changed during your effort that you didn't claim — \
                 too many to triage; skipping. Likely from another effort, formatter, \
                 or external activity.)\n"
            ));
        }
        if !r.unattributed_runs.is_empty() {
            out.push_str(
                "      These test/coverage/analysis runs happened during your effort but \
                 weren't attributed to you (a concurrent effort was open, so oxplow couldn't \
                 auto-assign them):\n",
            );
            for run in &r.unattributed_runs {
                out.push_str(&format!("        - {run}\n"));
            }
        }
    }
    out.push_str(
        "\nIf any are wrong, call `mcp__oxplow__amend_effort(effort_id, add_files, \
         remove_files, claim_runs, disclaim_runs)` to correct — `claim_runs` for runs \
         that were yours, `disclaim_runs` for ones that weren't. If your original \
         declaration was right (you reverted an edit, or another actor/effort produced \
         those changes/runs), no amend is needed — silent agreement is fine and the \
         prompt won't repeat.",
    );
    out
}

fn build_filed_but_didnt_ship_reason() -> String {
    "FILED BUT DIDN'T SHIP: you filed a `ready` task this turn but didn't open one as `in_progress` and didn't make any code edits. \
     If you meant to start the work, mark one in_progress and re-issue the edit. \
     If you meant to queue it for later, reply with that intent and the next turn picks it up."
        .into()
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
    use oxplow_domain::{
        AgentKind, Stream, StreamKind, Task, TaskActorKind, TaskId, TaskPriority, ThreadStatus,
        Timestamp,
    };

    #[test]
    fn format_run_descriptor_renders_summary_and_time() {
        // tsk266: the agent gets a recognizable line, not an opaque id.
        assert_eq!(
            format_run_descriptor(
                "run:47",
                Some("cargo test (419 passed, 0 failed)"),
                Some((10, 3))
            ),
            "run:47 — cargo test (419 passed, 0 failed) @ 10:03"
        );
        // Every piece is optional; the ref always leads so claim_runs can parse it.
        assert_eq!(format_run_descriptor("run:9", None, None), "run:9");
        assert_eq!(format_run_descriptor("run:9", Some("   "), None), "run:9");
    }

    #[test]
    fn run_summary_dispatches_per_detail_kind() {
        // tsk269: a coverage/analysis run renders as such, not a malformed test.
        let test = serde_json::json!({"command": "cargo test", "passed": 419, "failed": 0});
        assert_eq!(
            run_summary_from_detail("test-detail", &test).as_deref(),
            Some("cargo test (419 passed, 0 failed)")
        );
        let cov = serde_json::json!({"summaryPct": 83.4});
        assert_eq!(
            run_summary_from_detail("coverage-detail", &cov).as_deref(),
            Some("coverage (83% of changed lines)")
        );
        let analysis =
            serde_json::json!({"analyzer": "clippy", "errorCount": 0, "warningCount": 2});
        assert_eq!(
            run_summary_from_detail("analysis-detail", &analysis).as_deref(),
            Some("clippy (0 errors, 2 warnings)")
        );
        assert_eq!(run_summary_from_detail("other", &test), None);
    }

    /// Build an `AppCtx` over an in-memory DB for filing-claim tests.
    /// The session layer refuses non-git dirs, so init a repo first.
    fn test_ctx() -> (AppCtx, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "test"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let services = Arc::new(Services::in_memory(dir.path()).unwrap());
        let ctx = AppCtx {
            services,
            hook_token: Arc::new("t".into()),
            stop_state: Arc::new(Mutex::new(StopState::default())),
            role_state: Arc::new(Mutex::new(RoleState::default())),
            resume_state: Arc::new(Mutex::new(HashMap::new())),
        };
        (ctx, dir)
    }

    fn test_stream() -> Stream {
        Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Worktree,
            title: "feat".into(),
            branch: "feat".into(),
            branch_ref: "refs/heads/feat".into(),
            branch_source: "main".into(),
            worktree_path: "/repo/wt".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    fn test_thread(stream_id: StreamId, status: ThreadStatus) -> Thread {
        Thread {
            id: ThreadId::placeholder(),
            stream_id,
            title: "thread".into(),
            status,
            sort_index: 0,
            pane_target: "working".into(),
            agent: AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            archived_at: None,
        }
    }

    fn in_progress_task(thread_id: ThreadId) -> Task {
        Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread_id),
            parent_id: None,
            title: "work".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        }
    }

    /// The primary stream and its seeded active (writer) thread, which
    /// `Services::in_memory` creates via `ensure_primary` at boot.
    async fn primary_writer(ctx: &AppCtx) -> (Stream, Thread) {
        let stream = ctx.services.stream_store.primary().await.unwrap().unwrap();
        let writer = ctx
            .services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.status == ThreadStatus::Active)
            .expect("primary stream has a seeded active writer thread");
        (stream, writer)
    }

    #[tokio::test]
    async fn claim_filed_on_sibling_thread_satisfies_writer() {
        // tsk133: a task filed on a sibling thread A (queued) but
        // dispatched to the stream's active writer thread B must satisfy
        // filing enforcement for B — no manual move_task required.
        // Cross-thread dispatch within a stream just works because a
        // stream has exactly one active writer.
        let (ctx, _dir) = test_ctx();
        let (stream, writer_b) = primary_writer(&ctx).await;
        // Sibling queued thread A in the same stream.
        let sibling_a_id = ctx
            .services
            .thread_store
            .upsert(&test_thread(stream.id, ThreadStatus::Queued))
            .await
            .unwrap();
        // The in_progress claim lives on the SIBLING thread A, not B.
        ctx.services
            .task_store
            .insert(&in_progress_task(sibling_a_id))
            .await
            .unwrap();

        assert!(
            stream_has_in_progress_claim(&ctx, &writer_b).await,
            "an in_progress task on a sibling thread in the same stream must \
             satisfy the writer's filing guard"
        );
    }

    #[tokio::test]
    async fn claim_in_another_stream_does_not_satisfy() {
        // Scoping is per-stream, not global: an in_progress task in a
        // DIFFERENT stream must NOT unblock this stream's writer.
        let (ctx, _dir) = test_ctx();
        let (_stream, writer) = primary_writer(&ctx).await;
        let other_stream_id = ctx
            .services
            .stream_store
            .upsert(&test_stream())
            .await
            .unwrap();
        let other_thread_id = ctx
            .services
            .thread_store
            .upsert(&test_thread(other_stream_id, ThreadStatus::Active))
            .await
            .unwrap();
        ctx.services
            .task_store
            .insert(&in_progress_task(other_thread_id))
            .await
            .unwrap();

        assert!(
            !stream_has_in_progress_claim(&ctx, &writer).await,
            "an in_progress task in another stream must not satisfy this writer"
        );
    }

    #[tokio::test]
    async fn no_claim_anywhere_does_not_satisfy() {
        let (ctx, _dir) = test_ctx();
        let (_stream, writer) = primary_writer(&ctx).await;
        assert!(
            !stream_has_in_progress_claim(&ctx, &writer).await,
            "no in_progress task anywhere → guard not satisfied"
        );
    }

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
    fn out_of_worktree_edit_short_circuits_guards() {
        let wt = Path::new("/Users/x/proj");
        // Absolute path outside the worktree (e.g. the Claude Code plan file)
        // → short-circuit (true).
        assert!(edit_path_outside_worktree(
            Some("/Users/x/.claude/plans/p.md"),
            wt
        ));
        // In-worktree absolute → guards still apply (false).
        assert!(!edit_path_outside_worktree(
            Some("/Users/x/proj/src/a.rs"),
            wt
        ));
        // Relative path → falls through to the guards (false).
        assert!(!edit_path_outside_worktree(Some("src/a.rs"), wt));
        // No path → falls through (false).
        assert!(!edit_path_outside_worktree(None, wt));
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
    fn git_op_in_progress_detects_merge_head() {
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
        fs::write(tmp.path().join(".git/MERGE_HEAD"), b"deadbeef\n").unwrap();
        assert!(git_operation_in_progress(tmp.path()));
    }

    #[test]
    fn git_op_in_progress_detects_each_marker() {
        use std::fs;
        for marker in ["REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            let tmp = tempfile::TempDir::new().unwrap();
            fs::create_dir_all(tmp.path().join(".git")).unwrap();
            assert!(!git_operation_in_progress(tmp.path()));
            fs::write(tmp.path().join(".git").join(marker), b"deadbeef\n").unwrap();
            assert!(
                git_operation_in_progress(tmp.path()),
                "expected {marker} to count"
            );
        }
    }

    #[test]
    fn git_op_in_progress_follows_worktree_gitdir_pointer() {
        // In a secondary worktree, `.git` is a *file* pointing at the
        // real gitdir. The function must follow that pointer so a
        // mid-merge worktree still trips the carve-out.
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        let real_gitdir = tmp.path().join("real-gitdir");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(&real_gitdir).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real_gitdir.display()),
        )
        .unwrap();
        assert!(!git_operation_in_progress(&worktree));
        fs::write(real_gitdir.join("MERGE_HEAD"), b"x\n").unwrap();
        assert!(git_operation_in_progress(&worktree));
    }

    #[test]
    fn git_op_in_progress_no_dot_git_returns_false() {
        // Bare directory with no .git at all — function must not
        // panic and must report no-op.
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
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
    fn pre_tool_check_applies_only_to_worktree_mutating_tools() {
        // The four structured-edit tools are the only ones either guard
        // can deny — pre_tool_check must run for them.
        for t in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
            assert!(pre_tool_check_applies(t), "{t} must run the full check");
        }
        // Everything else short-circuits: both guards provably return None,
        // so the DB reads + git stats are skipped.
        for t in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "Task",
            "WebFetch",
            "WebSearch",
            "TodoWrite",
            "mcp__oxplow__create_task",
            "",
        ] {
            assert!(!pre_tool_check_applies(t), "{t} must short-circuit");
        }
    }

    #[test]
    fn pre_tool_check_gate_matches_canonical_guard_lists() {
        // Equivalence guard: the gate must admit exactly the union of the
        // two guards' tool sets, so narrowing the gate can never silently
        // drop a tool a guard would have denied.
        use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
        use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
        for t in WORKTREE_MUTATING_TOOLS
            .iter()
            .chain(ALWAYS_WRITE_INTENT_TOOL_NAMES.iter())
        {
            assert!(pre_tool_check_applies(t), "gate must admit guarded {t}");
        }
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
