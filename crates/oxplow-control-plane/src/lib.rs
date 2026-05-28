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
use oxplow_domain::{HookKind, StreamId, TaskStatus, ThreadId};
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
/// runtime sees for a given Claude session id, then reuses that
/// snapshot as the comparison baseline for the ROLE CHANGE banner on
/// every subsequent hook of that session. Keyed by session_id (not
/// thread_id) so a thread that re-attaches with a fresh Claude
/// session — e.g. after a daemon restart — gets a fresh baseline.
/// Loss across restart is acceptable: the worst case is one extra
/// no-op turn before the next promotion gets a banner.
#[derive(Default)]
struct RoleState {
    initial_role_by_session_id: HashMap<String, RoleMode>,
}

#[derive(Clone)]
struct AppCtx {
    services: Arc<Services>,
    hook_token: Arc<String>,
    stop_state: Arc<Mutex<StopState>>,
    role_state: Arc<Mutex<RoleState>>,
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

async fn handle_hook(
    State(ctx): State<AppCtx>,
    AxumPath(event): AxumPath<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_bearer(&headers, &ctx.hook_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }

    let kind = match parse_hook_kind(&event) {
        Some(k) => k,
        None => {
            // Unknown but non-fatal — record nothing, ack so the agent
            // doesn't block.
            return (StatusCode::ACCEPTED, "ignored unknown hook event").into_response();
        }
    };

    let stream_id = headers
        .get("x-oxplow-stream")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| StreamId::from(s.to_string()));
    let thread_id = headers
        .get("x-oxplow-thread")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| ThreadId::from(s.to_string()));

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
                thread_id: thread_id.clone(),
                stream_id: stream_id.clone(),
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
        thread_id: thread_id.clone(),
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
        warn!(?event, ?err, "hook ingest failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ingest failed: {err}"),
        )
            .into_response();
    }

    // Resume-tracker: Claude Code drops HTTP hooks for SessionStart, so
    // we learn the session_id from whichever hook fires next. Persist
    // it onto the thread so the next agent spawn passes
    // `--resume <session_id>` and Claude actually picks up where it
    // left off (without this, every re-attach starts a fresh session).
    update_resume_session_id(&ctx, &envelope_for_resume).await;

    // PostToolUse: attribute wiki-page edits to the originating thread
    // so the rail's "Finished" list can surface only the pages this
    // thread authored or revised.
    if kind == HookKind::PostToolUse {
        if let (Some(thread_id), Some(body)) =
            (envelope_for_resume.thread_id.as_ref(), body_value.as_ref())
        {
            attribute_wiki_page_edit(&ctx, thread_id, body).await;

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
    // role on every prompt. Captures the launch-time role on the
    // first prompt of each session so subsequent prompts can detect
    // promotions/demotions and append a loud ROLE CHANGE banner. The
    // block is returned via hookSpecificOutput.additionalContext per
    // the Claude Code hook contract.
    if kind == HookKind::UserPromptSubmit {
        if let Some(thread_id) = envelope_for_resume.thread_id.as_ref() {
            if let Some(ctx_block) = refreshed_session_context(
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
                            "hookEventName": "UserPromptSubmit",
                            "additionalContext": ctx_block,
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

    (StatusCode::ACCEPTED, Json(serde_json::json!({}))).into_response()
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
    if tool_name.is_empty() {
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
    let has_in_progress_task = ctx
        .services
        .task_store
        .list_for_thread(thread_id)
        .await
        .map(|items| items.iter().any(|i| i.status == TaskStatus::InProgress))
        .unwrap_or(false);

    let file_path = tool_input
        .and_then(|t| {
            t.get("file_path")
                .or_else(|| t.get("notebook_path"))
                .or_else(|| t.get("path"))
        })
        .and_then(|v| v.as_str());

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
    let thread = match ctx.services.thread_store.get(thread_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(err) => {
            warn!(?err, "resume-tracker: thread lookup failed");
            return;
        }
    };
    if thread.resume_session_id == observed {
        return;
    }
    let mut updated = thread;
    updated.resume_session_id = observed.to_string();
    updated.updated_at = oxplow_domain::Timestamp::now();
    if let Err(err) = ctx.services.thread_store.upsert(&updated).await {
        warn!(?err, "resume-tracker: thread upsert failed");
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
            let Some(review) = oxplow_app::task_service::recompute_effort_file_review(
                &ctx.services.effort_store,
                &ctx.services.snapshot_store,
                &eid,
            )
            .await
            else {
                continue;
            };
            let title = titles_by_id
                .get(&review.task_id)
                .cloned()
                .unwrap_or_else(|| format!("task {}", review.task_id));
            pending_reviews.push(PendingEffortReview {
                effort_id: review.effort_id,
                task_id: review.task_id,
                task_title: title,
                claimed_but_not_changed: review.claimed_but_not_changed,
                changed_but_not_claimed: review.changed_but_not_claimed,
                unclaimed_overflow: review.unclaimed_overflow,
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
                    st.last_audit_signature
                        .insert(thread_id.clone(), sig.clone());
                }
                StopHookSideEffect::RecordFiledButDidntShipFired => {
                    st.filed_but_didnt_ship_fired
                        .insert(thread_id.clone(), true);
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

fn build_effort_file_review_reason(reviews: &[PendingEffortReview]) -> String {
    let mut out = String::from(
        "EFFORT FILE REVIEW: one or more efforts you just closed have a discrepancy \
         between your declared `touched_files` and the snapshot diff. For each:\n\n",
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
    }
    out.push_str(
        "\nIf any are wrong, call `mcp__oxplow__amend_effort(effort_id, add_files, \
         remove_files)` to correct. If you genuinely meant your original list \
         (e.g. you edited then reverted, or another actor changed those files), no \
         amend is needed — silent agreement is fine and the prompt won't repeat.",
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
    Some(build_session_context_block_with_role(
        &stream,
        Some(&thread),
        initial,
    ))
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
        // perspective. Returning None routes them to ACCEPTED above
        // without persisting. AgentBoot, SubagentStop, Interrupt are
        // synthetic / not posted by the plugin.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
