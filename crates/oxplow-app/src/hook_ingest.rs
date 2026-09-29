//! Hook ingest: where every agent harness's activity enters oxplow.
//!
//! Claude/Codex/opencode hooks (via the control plane) and the ACP client
//! all hand a Claude-shaped [`HookEnvelope`] to [`HookIngestService::ingest`].
//! One envelope is **one transaction** (P3.3, tsk473): the state it changes
//! (the thread's resume session, the agent turn) and the `agent.*` events
//! that record it commit together, anchored to the thread's stream, its open
//! turn and its single open effort. Large bodies (tool input and output)
//! go into `event_content` by hash. Reactors on the event pump do the rest
//! (tool-call rows, effort claims, collection, token usage).
//!
//! Per kind:
//! - `UserPromptSubmit`: `agent.prompt.submitted`; opens a turn when none
//!   is open (`agent.turn.started`); status Running.
//! - `PreToolUse` / `PostToolUse`: `agent.tool.requested` (with the
//!   policy's decision) / `agent.tool.finished`, deduped by the harness's
//!   `tool_use_id` so a re-posted hook logs once.
//! - `Stop` / `Interrupt`: closes the open turns (`agent.turn.ended`);
//!   status Idle / AwaitingUser / Stopped; then the turn-end snapshot.
//! - `SessionStart` (a harness process starting or resuming a session —
//!   Claude's command hook, Codex's hook, the ACP client; not a `compact`):
//!   closes the turns the previous process left open (interrupted), logs
//!   `agent.session.started`, status Idle.
//! - A session id seen for the first time on a thread (on any kind)
//!   becomes its resume id and logs `agent.session.started` once.
//! - `SessionEnd`: `agent.session.ended`; `reason: clear` of the resume
//!   session clears the resume id.
//!
//! Agent status is the log: a thread's status is its newest
//! `agent.status.changed`, read and compared inside the same transaction
//! that logs a change. Announcements (`AgentStatusChanged`) go out under
//! one lock held from the transaction to the emit, so they reach the UI
//! in commit order.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::agent_stores::{
    activity_anchors_tx, close_turn_tx, last_status_tx, open_turn_ids_tx, open_turn_tx,
};
use oxplow_db::event_log_store::{append_unique_tx, EventCtx};
use oxplow_db::{event_content_store, Database};
use oxplow_domain::events::schema::{
    AgentPromptSubmitted, AgentPromptSubmittedV1, AgentSessionEnded, AgentSessionEndedV1,
    AgentSessionStarted, AgentSessionStartedV1, AgentStatusChanged, AgentStatusChangedV1,
    AgentToolFinished, AgentToolFinishedV1, AgentToolRequested, AgentToolRequestedV1, ContentRef,
    ToolDecision as Decision,
};
use oxplow_domain::refs::build::{thread_ref, turn_ref};
use oxplow_domain::{
    AgentKind, AgentStatus, AgentStatusState, AgentTurnId, DomainError, EventSchemaRegistry,
    HookKind, StreamId, ThreadId, Timestamp,
};

use crate::events::{EventBus, OxplowEvent};
use oxplow_domain::hook::TurnOutcome;

/// The Stop-body key a transport puts a turn's own token counts under
/// (ACP's prompt response); they ride `agent.turn.ended@2 { usage }`.
pub const TURN_USAGE_KEY: &str = "oxplow_turn_usage";

/// What the agent policy decided about a tool call (PreToolUse), carried
/// on the envelope so the log records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ToolDecision {
    pub allowed: bool,
    pub reason: Option<String>,
}

/// What the hook subprocess sends us.
///
/// The renderer / Claude Code emit JSON envelopes; the daemon receives
/// them and lands them here. `payload_json` is the verbatim envelope
/// minus the routing fields we hoist into typed columns.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HookEnvelope {
    pub kind: HookKind,
    pub thread_id: Option<ThreadId>,
    pub stream_id: Option<StreamId>,
    pub session_id: Option<String>,
    pub payload_json: String,
    /// Optional client-supplied prompt body for UserPromptSubmit so
    /// the agent_turn row carries the visible prompt text.
    pub prompt: Option<String>,
    /// PreToolUse only: the policy's verdict (`None` reads as allowed).
    #[serde(default)]
    pub decision: Option<ToolDecision>,
}

#[derive(Debug, Error)]
pub enum HookIngestError {
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

/// What one ingest did, for the request path that follows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestOutcome {
    /// The thread's open turn after the ingest (the one a prompt opened).
    pub turn: Option<AgentTurnId>,
    /// The turn a Stop / Interrupt closed.
    pub closed_turn: Option<AgentTurnId>,
}

/// What the transaction decided, applied after it commits.
#[derive(Default)]
struct Applied {
    turn: Option<AgentTurnId>,
    opened_turn: bool,
    closed_turn: Option<AgentTurnId>,
    status: Option<(AgentStatusState, Option<String>)>,
}

#[derive(Clone)]
pub struct HookIngestService {
    db: Database,
    schemas: Arc<EventSchemaRegistry>,
    /// Paths the tools name are made relative to the thread's worktree; a
    /// stream with none recorded works in the project directory.
    project_dir: PathBuf,
    /// Reads recent activity to derive a thread's status.
    log: oxplow_db::SqliteEventLogStore,
    /// Held from a status-deciding transaction to its announcement, so
    /// `AgentStatusChanged` reaches the UI in commit order.
    status_order: Arc<tokio::sync::Mutex<()>>,
    events: EventBus,
    pump: Option<Arc<crate::event_pump::EventPump>>,
    /// Snapshots the worktree when a turn ends (P2.3); `None` in bare tests.
    turn_snapshots: Option<Arc<dyn crate::turn_snapshots::TurnSnapshots>>,
}

impl HookIngestService {
    pub fn new(
        db: Database,
        schemas: Arc<EventSchemaRegistry>,
        project_dir: PathBuf,
        events: EventBus,
    ) -> Self {
        Self {
            log: oxplow_db::SqliteEventLogStore::new(db.clone(), schemas.clone()),
            db,
            schemas,
            project_dir,
            status_order: Arc::new(tokio::sync::Mutex::new(())),
            events,
            pump: None,
            turn_snapshots: None,
        }
    }

    /// Wake the event pump after each ingest, so reactors see the events.
    pub fn with_event_pump(mut self, pump: Arc<crate::event_pump::EventPump>) -> Self {
        self.pump = Some(pump);
        self
    }

    /// Take a `turn_end` snapshot whenever a turn closes.
    pub fn with_turn_snapshots(
        mut self,
        snapshots: Arc<dyn crate::turn_snapshots::TurnSnapshots>,
    ) -> Self {
        self.turn_snapshots = Some(snapshots);
        self
    }

    /// Record the envelope and drive the turn / status state machine.
    pub async fn ingest(&self, env: HookEnvelope) -> Result<IngestOutcome, HookIngestError> {
        let now = Timestamp::now();
        let mut outcome = IngestOutcome::default();
        let kind = env.kind;
        let Some(thread) = env.thread_id else {
            return Ok(outcome); // no thread: nothing to anchor a record to
        };

        let order = self.status_order.lock().await;
        let schemas = self.schemas.clone();
        let project_dir = self.project_dir.clone();
        let applied = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "hook_ingest");
                record_tx(tx, &ev, &project_dir, thread, &env, now)
            })
            .await?;

        // The activity log (the Hook events page) refetches on this.
        self.events.emit(OxplowEvent::HookEventsChanged);
        outcome.turn = applied.turn;
        outcome.closed_turn = applied.closed_turn;
        if applied.opened_turn || applied.closed_turn.is_some() {
            self.events
                .emit(OxplowEvent::AgentTurnsChanged { thread_id: thread });
        }
        match applied.status {
            Some((state, detail)) => self.announce(thread, state, detail),
            None => self.announce_derived_status(&thread, kind).await,
        }
        drop(order);
        if let Some(pump) = &self.pump {
            pump.wake();
        }
        // The status is out first: the snapshot is bookkeeping and must
        // not hold the UI on "running".
        if let (Some(snapshots), Some(turn)) = (&self.turn_snapshots, applied.closed_turn) {
            snapshots.take_turn_end(thread, turn).await;
        }
        Ok(outcome)
    }

    /// Log and announce a thread's status outside a hook (`await_user`,
    /// the ACP session's permission cards).
    pub async fn set_status(
        &self,
        thread: &ThreadId,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<(), HookIngestError> {
        let _order = self.status_order.lock().await;
        let schemas = self.schemas.clone();
        let (thread_c, detail_c) = (*thread, detail.clone());
        let logged = self
            .db
            .transaction(move |tx| {
                let ev = EventCtx::system(&schemas, "hook_ingest");
                let current = last_status_tx(tx, &schemas, thread_c)?;
                if !changed(current.as_ref(), state, detail_c.as_deref()) {
                    return Ok(false);
                }
                log_status_tx(tx, &ev, thread_c, state, detail_c.clone())?;
                Ok(true)
            })
            .await?;
        if logged {
            // The activity log shows the status change.
            self.events.emit(OxplowEvent::HookEventsChanged);
        }
        self.announce(*thread, state, detail);
        Ok(())
    }

    fn announce(&self, thread: ThreadId, state: AgentStatusState, detail: Option<String>) {
        self.events.emit(OxplowEvent::AgentStatusChanged {
            thread_id: thread,
            state,
            detail,
        });
    }

    /// Tool hooks set no status of their own, but they change what the
    /// renderer derives (an open `Task` keeps a thread working). Re-derive
    /// from the thread's logged activity and announce it, keeping an
    /// `await_user` that parked the thread this turn.
    async fn announce_derived_status(&self, thread: &ThreadId, kind: HookKind) {
        if !matches!(kind, HookKind::PreToolUse | HookKind::PostToolUse) {
            return;
        }
        let current = {
            let (schemas, thread) = (self.schemas.clone(), *thread);
            self.db
                .transaction(move |tx| last_status_tx(tx, &schemas, thread))
                .await
                .ok()
                .flatten()
        };
        let (state, detail) = match current {
            Some(s) if s.state == AgentStatusState::AwaitingUser => {
                (AgentStatusState::AwaitingUser, s.detail)
            }
            _ => {
                let recent = crate::agent_status_derive::recent_activity(&self.log, *thread)
                    .await
                    .unwrap_or_default();
                let derived =
                    crate::agent_status_derive::derive_thread_status(&recent, Timestamp::now());
                (derived, None)
            }
        };
        self.announce(*thread, state, detail);
    }
}

/// A timestamp as every table stores it (fixed-width RFC 3339).
fn ts_string(ts: Timestamp) -> String {
    serde_json::to_value(ts)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Whether moving to `state`/`detail` is a change from `current`.
fn changed(current: Option<&AgentStatus>, state: AgentStatusState, detail: Option<&str>) -> bool {
    match current {
        Some(c) => c.state != state || c.detail.as_deref() != detail,
        None => true,
    }
}

fn log_status_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    state: AgentStatusState,
    detail: Option<String>,
) -> Result<(), DomainError> {
    let env = ev
        .typed::<AgentStatusChanged>(&AgentStatusChangedV1 {
            thread: thread_ref(thread),
            state,
            detail,
        })
        .with_anchors(activity_anchors_tx(conn, thread)?)
        .with_subject([thread_ref(thread)]);
    ev.append(conn, &env)?;
    Ok(())
}

/// The thread row fields the ingest needs.
struct ThreadRow {
    resume_session_id: String,
    agent: AgentKind,
    worktree: PathBuf,
}

fn thread_row_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    project_dir: &Path,
) -> Result<Option<ThreadRow>, DomainError> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT th.resume_session_id, th.agent, COALESCE(s.worktree_path, '')
           FROM threads th LEFT JOIN streams s ON s.id = th.stream_id
          WHERE th.id = ?1",
        [thread.value()],
        |r| {
            let agent: String = r.get(1)?;
            let worktree: String = r.get(2)?;
            Ok(ThreadRow {
                resume_session_id: r.get(0)?,
                agent: serde_json::from_value(serde_json::Value::String(agent)).unwrap_or_default(),
                worktree: if worktree.is_empty() {
                    project_dir.to_path_buf()
                } else {
                    PathBuf::from(worktree)
                },
            })
        },
    )
    .optional()
    .map_err(oxplow_db::map_sql_err)
}

/// The envelope's state changes and events, in one transaction.
fn record_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    project_dir: &Path,
    thread: ThreadId,
    env: &HookEnvelope,
    now: Timestamp,
) -> Result<Applied, DomainError> {
    let Some(row) = thread_row_tx(conn, thread, project_dir)? else {
        return Ok(Applied::default()); // an unknown thread: the hook log only
    };
    let body: serde_json::Value = serde_json::from_str(&env.payload_json).unwrap_or_default();
    let session = env.session_id.as_deref().filter(|s| !s.is_empty());
    let mut applied = Applied::default();
    let mut status = None;
    let starts = starts_session(env.kind, &body);
    if starts {
        // The process that owned any open turn is gone: they end
        // interrupted, before the session start that resets the thread.
        applied.closed_turn = close_open_turns_tx(
            conn,
            ev,
            thread,
            Some("session restarted"),
            TurnOutcome::Interrupted,
            None,
            None,
        )?;
        status = Some((AgentStatusState::Idle, None));
    }
    if env.kind != HookKind::SessionEnd {
        if let Some(sid) = session {
            track_session_tx(conn, ev, thread, &row, sid, starts, now)?;
        }
    }
    applied.turn = open_turn_ids_tx(conn, thread)?.first().copied();
    match env.kind {
        HookKind::UserPromptSubmit => {
            let reprompt = applied.turn.is_some();
            if !reprompt {
                let prompt = env.prompt.as_deref().unwrap_or_default();
                applied.turn = Some(open_turn_tx(conn, ev, thread, prompt, session, now)?);
                applied.opened_turn = true;
            }
            let payload = AgentPromptSubmittedV1 {
                thread: thread_ref(thread),
                turn: applied.turn.map(turn_ref),
                session: session.map(str::to_string),
                reprompt,
            };
            let env = ev
                .typed::<AgentPromptSubmitted>(&payload)
                .with_anchors(activity_anchors_tx(conn, thread)?)
                .with_subject([applied
                    .turn
                    .map(turn_ref)
                    .unwrap_or_else(|| thread_ref(thread))]);
            ev.append(conn, &env)?;
            status = Some((AgentStatusState::Running, None));
        }
        HookKind::PreToolUse | HookKind::PostToolUse => {
            log_tool_tx(conn, ev, thread, &row.worktree, env, &body, session)?;
        }
        HookKind::Stop | HookKind::Interrupt => {
            let (answer, outcome) = if env.kind == HookKind::Stop {
                (None, TurnOutcome::Completed)
            } else {
                (Some("interrupted"), TurnOutcome::Interrupted)
            };
            let transcript = body.get("transcript_path").and_then(|p| p.as_str());
            // Counts a harness reported with the turn itself (ACP).
            let usage: Option<oxplow_domain::events::schema::TurnUsage> = body
                .get(TURN_USAGE_KEY)
                .and_then(|u| serde_json::from_value(u.clone()).ok());
            applied.closed_turn =
                close_open_turns_tx(conn, ev, thread, answer, outcome, transcript, usage)?;
            applied.turn = None;
            status = Some(if env.kind == HookKind::Interrupt {
                (AgentStatusState::Stopped, Some("interrupt".to_string()))
            } else {
                stop_status(
                    &env.payload_json,
                    last_status_tx(conn, ev.schemas, thread)?.as_ref(),
                )
            });
        }
        HookKind::SessionEnd => {
            if let Some(sid) = session {
                end_session_tx(conn, ev, thread, &row, sid, &body, now)?;
            }
        }
        HookKind::SessionStart => {} // handled above
    }
    if let Some((state, detail)) = &status {
        if changed(
            last_status_tx(conn, ev.schemas, thread)?.as_ref(),
            *state,
            detail.as_deref(),
        ) {
            log_status_tx(conn, ev, thread, *state, detail.clone())?;
        }
    }
    applied.status = status;
    Ok(applied)
}

/// A Stop parks the thread on the person when the agent asked them
/// something this turn — a sentinel on this payload, or an `await_user`
/// the MCP tool already recorded (the real Stop payload carries none, and
/// a fresh prompt clears it first) — else it goes idle.
fn stop_status(payload: &str, current: Option<&AgentStatus>) -> (AgentStatusState, Option<String>) {
    let awaiting = current.is_some_and(|s| s.state == AgentStatusState::AwaitingUser);
    if payload_signals_await_user(payload) || awaiting {
        let question =
            await_user_question(payload).or_else(|| current.and_then(|s| s.detail.clone()));
        (AgentStatusState::AwaitingUser, question)
    } else {
        (AgentStatusState::Idle, None)
    }
}

/// Whether this hook is a harness process starting (or resuming) a
/// session: a `SessionStart`, except the one a context compaction posts
/// mid-turn (Claude's `source: "compact"` keeps the same process and turn).
fn starts_session(kind: HookKind, body: &serde_json::Value) -> bool {
    kind == HookKind::SessionStart && body.get("source").and_then(|s| s.as_str()) != Some("compact")
}

/// Close every open turn on the thread; returns the newest one closed,
/// which owns the turn-end snapshot.
fn close_open_turns_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    answer: Option<&str>,
    outcome: TurnOutcome,
    transcript: Option<&str>,
    usage: Option<oxplow_domain::events::schema::TurnUsage>,
) -> Result<Option<AgentTurnId>, DomainError> {
    let mut newest = None;
    // Newest first.
    for id in open_turn_ids_tx(conn, thread)? {
        if close_turn_tx(conn, ev, id, answer, outcome, transcript, usage.clone())?.is_some()
            && newest.is_none()
        {
            newest = Some(id);
        }
    }
    Ok(newest)
}

/// Log `agent.session.started` and make the id the thread's resume id, so
/// the next spawn passes `--resume <id>`. It is logged the first time any
/// hook carries the id (Claude posts no HTTP SessionStart for a fresh
/// session), and again on every process start (`starts`) after that — a
/// resume is a start, and it resets the thread's derived status.
fn track_session_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    row: &ThreadRow,
    session: &str,
    starts: bool,
    now: Timestamp,
) -> Result<(), DomainError> {
    let env = ev
        .typed::<AgentSessionStarted>(&AgentSessionStartedV1 {
            session: session.to_string(),
            thread: thread_ref(thread),
            harness: row.agent,
            resumed: row.resume_session_id == session,
        })
        .with_anchors(activity_anchors_tx(conn, thread)?)
        .with_subject([thread_ref(thread)]);
    let first = env
        .clone()
        .with_dedupe_key(format!("session:{session}:started"));
    if !append_unique_tx(conn, ev.schemas, &first)? && starts {
        ev.append(conn, &env)?;
    }
    if row.resume_session_id != session {
        conn.execute(
            "UPDATE threads SET resume_session_id = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![thread.value(), session, ts_string(now)],
        )
        .map_err(oxplow_db::map_sql_err)?;
    }
    Ok(())
}

/// `SessionEnd`: log it, and drop the resume id only when an explicit
/// `/clear` ended exactly the session it points at — a normal exit keeps
/// it (a restart should resume), and clearing a stale session must not
/// wipe a newer one. (Claude starts the post-clear session with no HTTP
/// hook, so without this a restart would resurrect the cleared session.)
fn end_session_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    row: &ThreadRow,
    session: &str,
    body: &serde_json::Value,
    now: Timestamp,
) -> Result<(), DomainError> {
    let reason = body.get("reason").and_then(|r| r.as_str());
    let env = ev
        .typed::<AgentSessionEnded>(&AgentSessionEndedV1 {
            session: session.to_string(),
            thread: thread_ref(thread),
            reason: reason.map(str::to_string),
        })
        .with_anchors(activity_anchors_tx(conn, thread)?)
        .with_subject([thread_ref(thread)]);
    ev.append(conn, &env)?;
    if reason == Some("clear") && row.resume_session_id == session {
        conn.execute(
            "UPDATE threads SET resume_session_id = '', updated_at = ?2 WHERE id = ?1",
            rusqlite::params![thread.value(), ts_string(now)],
        )
        .map_err(oxplow_db::map_sql_err)?;
    }
    Ok(())
}

/// `agent.tool.requested` / `agent.tool.finished` for a tool hook, with
/// the input (and output) stored by hash.
fn log_tool_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    worktree: &Path,
    env: &HookEnvelope,
    body: &serde_json::Value,
    session: Option<&str>,
) -> Result<(), DomainError> {
    let Some(parts) = crate::tool_calls::parse_tool_call(&env.payload_json, worktree) else {
        return Ok(()); // no tool name: nothing to record
    };
    let content = |key: &str| -> Result<Option<ContentRef>, DomainError> {
        match body.get(key) {
            Some(v) if !v.is_null() => {
                let bytes =
                    serde_json::to_vec(v).map_err(|e| DomainError::Invalid(e.to_string()))?;
                event_content_store::put_tx(conn, "agent", &bytes).map(Some)
            }
            _ => Ok(None),
        }
    };
    let tool_use = body.get("tool_use_id").and_then(|t| t.as_str());
    let dedupe =
        |phase: &str| tool_use.map(|id| format!("{}:{id}:{phase}", session.unwrap_or("-")));
    let anchors = activity_anchors_tx(conn, thread)?;
    let subject = anchors
        .turn_id
        .map(|t| turn_ref(AgentTurnId::new(t)))
        .unwrap_or_else(|| thread_ref(thread));
    let envelope = if env.kind == HookKind::PreToolUse {
        let decision = env.decision.clone().unwrap_or(ToolDecision {
            allowed: true,
            reason: None,
        });
        ev.typed::<AgentToolRequested>(&AgentToolRequestedV1 {
            tool: parts.tool,
            path: parts.path,
            detail: parts.detail,
            input: content("tool_input")?,
            decision: if decision.allowed {
                Decision::Allowed
            } else {
                Decision::Denied
            },
            reason: decision.reason,
        })
        .with_dedupe_key_opt(dedupe("requested"))
    } else {
        let exit_code =
            crate::collection::parse_bash_post_tool(&env.payload_json).and_then(|b| b.exit_code);
        ev.typed::<AgentToolFinished>(&AgentToolFinishedV1 {
            tool: parts.tool,
            path: parts.path,
            detail: parts.detail,
            ok: parts.ok,
            exit_code,
            input: content("tool_input")?,
            output: content("tool_response")?,
        })
        .with_dedupe_key_opt(dedupe("finished"))
    };
    let envelope = envelope.with_anchors(anchors).with_subject([subject]);
    append_unique_tx(conn, ev.schemas, &envelope)?;
    Ok(())
}

/// Heuristic: did the agent call mcp__oxplow__await_user during the
/// turn? Encoded as a sentinel in the payload so we don't have to
/// thread state through the pipeline.
fn payload_signals_await_user(payload: &str) -> bool {
    if !payload.contains("await_user") {
        return false;
    }
    // Cheap substring match — a full JSON parse on every Stop is
    // overkill since we control the sentinel writer.
    let lower = payload.to_ascii_lowercase();
    lower.contains("\"await_user\":true") || lower.contains("await_user_called")
}

/// Extract the question text from an await_user sentinel payload. Returns
/// None when the payload isn't an await_user signal or carries no
/// (non-empty) `question` field — callers then fall back to any question
/// already stored on `agent_status.detail`.
fn await_user_question(payload: &str) -> Option<String> {
    if !payload_signals_await_user(payload) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let q = v.get("question")?.as_str()?.trim();
    (!q.is_empty()).then(|| q.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::AgentTurnStore as _;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamKind, Thread, ThreadStatus};
    use serde_json::json;

    async fn fixture() -> (HookIngestService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let svc = HookIngestService::new(
            db,
            Arc::new(oxplow_domain::EventSchemaRegistry::core()),
            std::path::PathBuf::from("/p"),
            EventBus::new(),
        );
        (svc, t.id)
    }

    /// A new service over the same database: a restarted daemon.
    fn restarted(svc: &HookIngestService) -> HookIngestService {
        HookIngestService::new(
            svc.db.clone(),
            svc.schemas.clone(),
            svc.project_dir.clone(),
            EventBus::new(),
        )
    }

    #[tokio::test]
    async fn set_status_logs_once_and_refreshes_the_activity_log() {
        let (svc, tid) = fixture().await;
        let mut rx = svc.events.subscribe();
        for _ in 0..2 {
            svc.set_status(&tid, AgentStatusState::AwaitingUser, Some("A?".into()))
                .await
                .unwrap();
        }
        let mut refreshes = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, OxplowEvent::HookEventsChanged) {
                refreshes += 1;
            }
        }
        // The second call changed nothing: no second event, no refetch.
        assert_eq!(refreshes, 1);
        let events = logged(&svc).await;
        assert_eq!(of_type(&events, "agent.status.changed").len(), 1);
    }

    /// The thread's status as a freshly started daemon would read it.
    async fn status(svc: &HookIngestService, tid: ThreadId) -> Option<AgentStatus> {
        use oxplow_domain::stores::AgentStatusStore as _;
        oxplow_db::SqliteAgentStatusStore::new(svc.db.clone(), svc.schemas.clone())
            .get(&tid)
            .await
            .unwrap()
    }

    fn turns(svc: &HookIngestService) -> oxplow_db::SqliteAgentTurnStore {
        oxplow_db::SqliteAgentTurnStore::new(svc.db.clone())
    }

    /// Every event in the log, oldest first.
    async fn logged(svc: &HookIngestService) -> Vec<oxplow_domain::StoredEvent> {
        oxplow_db::SqliteEventLogStore::new(svc.db.clone(), svc.schemas.clone())
            .read_after(0, 1000)
            .await
            .unwrap()
    }

    fn of_type<'a>(
        events: &'a [oxplow_domain::StoredEvent],
        ty: &str,
    ) -> Vec<&'a oxplow_domain::StoredEvent> {
        events
            .iter()
            .filter(|e| e.envelope.event_type == ty)
            .collect()
    }

    fn hook(
        kind: HookKind,
        tid: ThreadId,
        session: Option<&str>,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(tid),
            stream_id: None,
            session_id: session.map(str::to_string),
            payload_json: body.to_string(),
            prompt: body
                .get("prompt")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            decision: None,
        }
    }

    async fn open_effort(svc: &HookIngestService) -> i64 {
        svc.db
            .transaction(|c| {
                c.execute(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                       VALUES ('work_item:linear:ENG-1', 1, '2026-01-01T00:00:00.000000Z')",
                    [],
                )
                .map_err(|e| DomainError::Storage(e.to_string()))?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// P3.3 (tsk473): a finished tool call is an `agent.tool.finished`
    /// anchored to stream, thread, turn and effort, with its input and
    /// output stored by hash and its path made worktree-relative.
    #[tokio::test]
    async fn a_finished_tool_is_logged_with_four_anchors_and_its_content() {
        let (svc, tid) = fixture().await;
        let effort = open_effort(&svc).await;
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("s1"),
            json!({"prompt": "go"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::PostToolUse,
            tid,
            Some("s1"),
            json!({
                "tool_name": "Edit",
                "tool_use_id": "tu1",
                "tool_input": {"file_path": "/p/src/a.rs", "old_string": "a", "new_string": "b"},
                "tool_response": {"filePath": "/p/src/a.rs"}
            }),
        ))
        .await
        .unwrap();
        let events = logged(&svc).await;
        let finished = of_type(&events, "agent.tool.finished");
        assert_eq!(finished.len(), 1);
        let e = &finished[0].envelope;
        let turn = turns(&svc).list_open(&tid).await.unwrap()[0].id.value();
        assert_eq!(e.anchors.stream_id.map(|s| s.value()), Some(1));
        assert_eq!(e.anchors.thread_id, Some(tid));
        assert_eq!(e.anchors.turn_id, Some(turn));
        assert_eq!(e.anchors.effort_id.map(|e| e.value()), Some(effort));
        assert_eq!(e.payload["tool"], "Edit");
        assert_eq!(e.payload["path"], "src/a.rs");
        assert_eq!(e.payload["ok"], true);
        let hash = e.payload["input"]["hash"].as_str().unwrap().to_string();
        let input = oxplow_db::event_content_store::read(&svc.db, &hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&input).unwrap()["new_string"],
            "b"
        );
        assert!(e.payload["output"]["hash"].is_string());
        // The same hook posted again (a harness retry) logs nothing new.
        svc.ingest(hook(
            HookKind::PostToolUse,
            tid,
            Some("s1"),
            json!({"tool_name": "Edit", "tool_use_id": "tu1", "tool_input": {"file_path": "src/a.rs"}}),
        ))
        .await
        .unwrap();
        assert_eq!(of_type(&logged(&svc).await, "agent.tool.finished").len(), 1);
    }

    /// A PreToolUse the policy refused is logged with its decision.
    #[tokio::test]
    async fn a_denied_request_is_logged_as_denied() {
        let (svc, tid) = fixture().await;
        let mut env = hook(
            HookKind::PreToolUse,
            tid,
            None,
            json!({"tool_name": "Write", "tool_input": {"file_path": "src/b.rs", "content": "x"}}),
        );
        env.decision = Some(ToolDecision {
            allowed: false,
            reason: Some("no effort is open".into()),
        });
        svc.ingest(env).await.unwrap();
        let events = logged(&svc).await;
        let requested = of_type(&events, "agent.tool.requested");
        assert_eq!(requested.len(), 1);
        let p = &requested[0].envelope.payload;
        assert_eq!(
            (
                p["tool"].as_str(),
                p["decision"].as_str(),
                p["reason"].as_str()
            ),
            (Some("Write"), Some("denied"), Some("no effort is open"))
        );
    }

    /// A session id seen for the first time starts a session (once), and
    /// becomes the thread's resume id; `/clear` of that session ends it.
    #[tokio::test]
    async fn sessions_are_tracked_by_the_ingest() {
        let (svc, tid) = fixture().await;
        for _ in 0..2 {
            svc.ingest(hook(
                HookKind::UserPromptSubmit,
                tid,
                Some("s1"),
                json!({"prompt": "go"}),
            ))
            .await
            .unwrap();
        }
        let started = of_type(&logged(&svc).await, "agent.session.started")
            .into_iter()
            .map(|e| e.envelope.payload.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            started,
            vec![
                json!({"session": "s1", "thread": "thread:thr1", "harness": "claude", "resumed": false})
            ]
        );
        let resume = || async {
            let db = svc.db.clone();
            db.transaction(move |c| {
                c.query_row(
                    "SELECT resume_session_id FROM threads WHERE id = 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap()
        };
        assert_eq!(resume().await, "s1");
        // An exit that isn't a clear keeps the resume id; a clear of a
        // stale session keeps it too; a clear of the resume session drops it.
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("s1"),
            json!({"reason": "other"}),
        ))
        .await
        .unwrap();
        assert_eq!(resume().await, "s1");
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("old"),
            json!({"reason": "clear"}),
        ))
        .await
        .unwrap();
        assert_eq!(resume().await, "s1");
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("s1"),
            json!({"reason": "clear"}),
        ))
        .await
        .unwrap();
        assert_eq!(resume().await, "");
        let ended = of_type(&logged(&svc).await, "agent.session.ended").len();
        assert_eq!(ended, 3, "every end is logged, s1's exit and its clear");
    }

    /// Every prompt is logged — one inside an open turn is a re-prompt —
    /// and status moves are logged once per transition.
    #[tokio::test]
    async fn prompts_and_status_transitions_are_logged() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            None,
            json!({"prompt": "a"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            None,
            json!({"prompt": "b"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::Stop,
            tid,
            None,
            json!({"transcript_path": "/t.jsonl"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(HookKind::Stop, tid, None, json!({})))
            .await
            .unwrap();
        let events = logged(&svc).await;
        let reprompts: Vec<bool> = of_type(&events, "agent.prompt.submitted")
            .iter()
            .map(|e| e.envelope.payload["reprompt"].as_bool().unwrap())
            .collect();
        assert_eq!(reprompts, vec![false, true]);
        let states: Vec<String> = of_type(&events, "agent.status.changed")
            .iter()
            .map(|e| e.envelope.payload["state"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(states, vec!["running", "idle"]);
        let ended = of_type(&events, "agent.turn.ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].envelope.v, 2);
        assert_eq!(ended[0].envelope.payload["transcript_path"], "/t.jsonl");
    }

    #[tokio::test]
    async fn turn_open_and_close_emit_agent_turns_changed() {
        // The Work panel renders open turns as live rows; it needs an
        // event on every open/close to refetch without polling.
        let (svc, tid) = fixture().await;
        let mut rx = svc.events.subscribe();
        let drain_turns = |rx: &mut tokio::sync::broadcast::Receiver<OxplowEvent>| {
            let mut n = 0;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, OxplowEvent::AgentTurnsChanged { .. }) {
                    n += 1;
                }
            }
            n
        };
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
            decision: None,
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 1, "open must emit AgentTurnsChanged");
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 1, "close must emit AgentTurnsChanged");
        // A Stop with nothing open closes nothing — no event.
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        assert_eq!(drain_turns(&mut rx), 0, "no-op close must stay quiet");
    }

    #[tokio::test]
    async fn user_prompt_opens_turn_and_marks_running() {
        let (svc, tid) = fixture().await;
        let env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: Some("sess".into()),
            payload_json: "{}".into(),
            prompt: Some("do the thing".into()),
            decision: None,
        };
        svc.ingest(env).await.unwrap();
        // Spot-check via stores.
        let turns = turns(&svc).list_open(&tid).await.unwrap();
        assert_eq!(turns.len(), 1);
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::Running);
    }

    #[tokio::test]
    async fn stop_closes_turn_and_marks_idle() {
        let (svc, tid) = fixture().await;
        // Open a turn first.
        let prompt_env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
            decision: None,
        };
        svc.ingest(prompt_env).await.unwrap();
        let stop = HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        };
        svc.ingest(stop).await.unwrap();
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
    }

    #[tokio::test]
    async fn stop_with_await_user_signal_marks_awaiting() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
            decision: None,
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: r#"{"await_user":true}"#.into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
    }

    #[tokio::test]
    async fn interrupt_closes_open_turn() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
            decision: None,
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Interrupt,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::Stopped);
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn stop_without_open_turn_still_marks_idle() {
        // Out-of-order: a Stop arriving with no open turn (e.g. after
        // a daemon restart dropped the in-memory turn, or a duplicate
        // Stop) must not error — it just lands the status transition.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
    }

    #[tokio::test]
    async fn envelope_without_thread_id_persists_event_only() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: None,
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("orphan".into()),
            decision: None,
        })
        .await
        .unwrap();
        // No turn opened, no status row created — the thread-scoped
        // state machine never ran.
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
        assert!(status(&svc, tid).await.is_none());
    }

    #[tokio::test]
    async fn reprompt_while_turn_open_does_not_open_second_turn() {
        let (svc, tid) = fixture().await;
        for prompt in ["first", "mid-turn re-prompt"] {
            svc.ingest(HookEnvelope {
                kind: HookKind::UserPromptSubmit,
                thread_id: Some(tid),
                stream_id: None,
                session_id: None,
                payload_json: "{}".into(),
                prompt: Some(prompt.into()),
                decision: None,
            })
            .await
            .unwrap();
        }
        let open = turns(&svc).list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].prompt, "first");
    }

    /// The derived status the rail would show for the thread now.
    async fn derived(svc: &HookIngestService, tid: ThreadId) -> AgentStatusState {
        let recent = crate::agent_status_derive::recent_activity(&svc.log, tid)
            .await
            .unwrap();
        crate::agent_status_derive::derive_thread_status(&recent, Timestamp::now())
    }

    #[tokio::test]
    async fn a_resumed_session_resets_a_turn_that_died_without_a_stop() {
        // A turn dies on an API error (no Stop); the pane restarts with
        // `--resume s1`. The session start closes the dead process's turn
        // and the thread reads idle, not running-then-stalled.
        let (svc, tid) = fixture().await;
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("s1"),
            json!({"prompt": "p"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::PreToolUse,
            tid,
            Some("s1"),
            json!({"tool_name": "Bash", "tool_use_id": "u1"}),
        ))
        .await
        .unwrap();
        assert_eq!(derived(&svc, tid).await, AgentStatusState::Running);
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s1"),
            json!({"source": "resume"}),
        ))
        .await
        .unwrap();
        assert_eq!(derived(&svc, tid).await, AgentStatusState::Idle);
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Idle
        );
        let events = logged(&svc).await;
        let started = of_type(&events, "agent.session.started");
        assert_eq!(started.len(), 2, "first sighting, then the resume");
        assert_eq!(started[1].envelope.payload["resumed"], true);

        // A second restart of the same session logs again.
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s1"),
            json!({"source": "resume"}),
        ))
        .await
        .unwrap();
        assert_eq!(
            of_type(&logged(&svc).await, "agent.session.started").len(),
            3
        );
    }

    #[tokio::test]
    async fn a_compaction_mid_turn_is_not_a_session_start() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("s1"),
            json!({"prompt": "p"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s1"),
            json!({"source": "compact"}),
        ))
        .await
        .unwrap();
        assert_eq!(turns(&svc).list_open(&tid).await.unwrap().len(), 1);
        assert_eq!(derived(&svc, tid).await, AgentStatusState::Running);
        assert_eq!(
            of_type(&logged(&svc).await, "agent.session.started").len(),
            1
        );
    }

    #[tokio::test]
    async fn a_session_start_for_a_new_id_logs_it_once() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s2"),
            json!({"source": "startup"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("s2"),
            json!({"prompt": "p"}),
        ))
        .await
        .unwrap();
        let events = logged(&svc).await;
        let started = of_type(&events, "agent.session.started");
        assert_eq!(started.len(), 1, "the prompt is not a second sighting");
        assert_eq!(started[0].envelope.payload["resumed"], false);
    }

    #[tokio::test]
    async fn each_end_of_a_resumed_session_is_logged() {
        let (svc, tid) = fixture().await;
        for _ in 0..2 {
            svc.ingest(hook(
                HookKind::SessionStart,
                tid,
                Some("s1"),
                json!({"source": "resume"}),
            ))
            .await
            .unwrap();
            svc.ingest(hook(
                HookKind::SessionEnd,
                tid,
                Some("s1"),
                json!({"reason": "exit"}),
            ))
            .await
            .unwrap();
        }
        assert_eq!(of_type(&logged(&svc).await, "agent.session.ended").len(), 2);
    }

    #[tokio::test]
    async fn real_stop_preserves_awaiting_user_set_by_mcp() {
        // The `await_user` MCP tool flips agent_status to AwaitingUser
        // (question as detail) mid-turn. The real Claude Stop that
        // follows carries no await_user sentinel — it must NOT clobber
        // that state back to Idle, or the rail "awaiting you" dot would
        // vanish the instant the turn ends.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
            decision: None,
        })
        .await
        .unwrap();
        // The MCP await_user call parks the thread; then the daemon
        // restarts before the Stop lands — nothing held in memory survives.
        svc.set_status(
            &tid,
            AgentStatusState::AwaitingUser,
            Some("Ship A or B?".into()),
        )
        .await
        .unwrap();
        let svc = restarted(&svc);
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
        assert_eq!(status.detail.as_deref(), Some("Ship A or B?"));
    }

    #[tokio::test]
    async fn post_tool_use_does_not_clobber_awaiting_user() {
        // await_user set AwaitingUser (with a question). A PostToolUse
        // that follows — e.g. the await_user tool call's own PostToolUse
        // — must NOT flicker the rail dot off "awaiting you": the derive
        // can't see the synthetic marker and would return Running.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
            decision: None,
        })
        .await
        .unwrap();
        svc.set_status(
            &tid,
            AgentStatusState::AwaitingUser,
            Some("Pick A or B?".into()),
        )
        .await
        .unwrap();
        let mut rx = svc.events.subscribe();
        svc.ingest(HookEnvelope {
            kind: HookKind::PostToolUse,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        let mut emitted = None;
        while let Ok(ev) = rx.try_recv() {
            if let OxplowEvent::AgentStatusChanged { state, detail, .. } = ev {
                emitted = Some((state, detail));
            }
        }
        let (state, detail) = emitted.expect("PostToolUse should emit AgentStatusChanged");
        assert_eq!(state, AgentStatusState::AwaitingUser);
        assert_eq!(detail.as_deref(), Some("Pick A or B?"));
    }

    #[tokio::test]
    async fn stop_await_user_signal_carries_question() {
        // A Stop payload carrying the sentinel + a question lands the
        // question on detail so the renderer can show it in the tooltip.
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            session_id: None,
            payload_json: r#"{"await_user":true,"question":"Pick A or B"}"#.into(),
            prompt: None,
            decision: None,
        })
        .await
        .unwrap();
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::AwaitingUser);
        assert_eq!(status.detail.as_deref(), Some("Pick A or B"));
    }

    #[test]
    fn await_user_payload_detection() {
        assert!(payload_signals_await_user(r#"{"await_user":true}"#));
        assert!(payload_signals_await_user(r#"{"x":"await_user_called"}"#));
        assert!(!payload_signals_await_user(r#"{}"#));
        assert!(!payload_signals_await_user(r#"{"await_user":false}"#));
    }

    #[test]
    fn await_user_question_extraction() {
        assert_eq!(
            await_user_question(r#"{"await_user":true,"question":"Pick A or B"}"#).as_deref(),
            Some("Pick A or B")
        );
        // Sentinel present but no question → None (caller falls back to
        // whatever detail the MCP tool already stored).
        assert_eq!(await_user_question(r#"{"await_user":true}"#), None);
        // Blank question → None.
        assert_eq!(
            await_user_question(r#"{"await_user":true,"question":"  "}"#),
            None
        );
        // Not an await_user payload → None.
        assert_eq!(await_user_question(r#"{"question":"x"}"#), None);
    }
}
