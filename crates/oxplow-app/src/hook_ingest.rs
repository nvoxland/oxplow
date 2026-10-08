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
//! - `SessionEnd`: closes the turns that session left open (interrupted,
//!   "session ended" — an exit mid-turn sends no Stop, tsk449) with its
//!   transcript, so their tokens are theirs (tsk924), status
//!   Stopped when it closed one; `agent.session.ended`; `reason: clear` of
//!   the resume session clears the resume id.
//!
//! Agent status is the log: a thread's status is its newest
//! `agent.status.changed`, read and compared inside the same transaction
//! that logs a change. Announcements (`AgentStatusChanged`) go out under
//! one lock held from the transaction to the emit, so they reach the UI
//! in commit order.

use oxplow_domain::vocabulary::VocabularyHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::agent_stores::{
    activity_anchors_tx, close_turn_tx, last_status_tx, log_status_tx, open_harness_turn_ids_tx,
    open_turn_ids_in_tx, open_turn_tx, TurnEnd,
};
use oxplow_db::event_log_store::{append_unique_tx, EventCtx};
use oxplow_db::{event_content_store, Database};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::events::schema::{
    AgentPromptSubmitted, AgentPromptSubmittedV1, AgentSessionEnded, AgentSessionEndedV1,
    AgentSessionStarted, AgentSessionStartedV2, AgentToolFinished, AgentToolFinishedV2,
    AgentToolRequested, AgentToolRequestedV2, ContentRef, ToolDecision as Decision,
};
use oxplow_domain::refs::build::{thread_ref, turn_ref};
use oxplow_domain::{
    AgentStatus, AgentStatusState, AgentTurnId, DomainError, HookKind, StreamId, ThreadId,
    Timestamp,
};

use crate::events::{EventBus, OxplowEvent};
use oxplow_domain::hook::TurnOutcome;

/// The Stop-body key a transport puts a turn's own token counts under
/// (ACP's prompt response); they ride `agent.turn.ended@2 { usage }`.
pub const TURN_USAGE_KEY: &str = "oxplow_turn_usage";

/// The Stop-body key holding the agent's final message for the turn
/// (Claude's own field; ACP sends its transcript's last agent message
/// under it): kept as the turn's `answer`.
pub const LAST_ASSISTANT_MESSAGE: &str = "last_assistant_message";

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
    /// The agent session it came from (`X-Oxplow-Session`, the ACP host,
    /// the UI's interrupt), when the sender knows it.
    #[serde(default)]
    pub agent_session_id: Option<oxplow_domain::AgentSessionId>,
    /// The harness's own session id.
    pub session_id: Option<String>,
    pub payload_json: String,
    /// UserPromptSubmit: the text the person submitted — stored with its
    /// `agent.prompt.submitted` (every prompt, a re-prompt too) and on the
    /// turn it opens.
    pub prompt: Option<String>,
    /// PreToolUse only: the policy's verdict (`None` reads as allowed).
    #[serde(default)]
    pub decision: Option<ToolDecision>,
    /// A tool hook's call in oxplow's vocabulary, when the sender mapped it
    /// (the control plane, with the hook's harness; the ACP host). `None`:
    /// the ingest maps the body with its session's harness. In-process
    /// only: an envelope that crosses a wire carries none.
    #[serde(skip)]
    #[specta(skip)]
    pub tool: Option<ToolUse>,
}

#[derive(Debug, Error)]
pub enum HookIngestError {
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

/// What one ingest did, for the request path that follows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestOutcome {
    /// The turn a Stop / Interrupt closed.
    pub closed_turn: Option<AgentTurnId>,
    /// The harness of the agent session the hook came from (its registry
    /// key): what renders the answer. `None` when no session claims it.
    pub harness: Option<String>,
}

/// What the transaction decided, applied after it commits.
#[derive(Default)]
struct Applied {
    /// The agent session the hook came from.
    session: Option<oxplow_domain::AgentSessionId>,
    /// Its harness's registry key.
    harness: Option<String>,
    turn: Option<AgentTurnId>,
    opened_turn: bool,
    closed_turn: Option<AgentTurnId>,
    status: Option<(AgentStatusState, Option<String>)>,
}

#[derive(Clone)]
pub struct HookIngestService {
    db: Database,
    vocabulary: VocabularyHandle,
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
    /// Maps a tool hook's body the sender didn't map, by its session's
    /// harness.
    harnesses: HarnessRegistry,
}

impl HookIngestService {
    pub fn new(
        db: Database,
        vocabulary: VocabularyHandle,
        project_dir: PathBuf,
        events: EventBus,
        harnesses: HarnessRegistry,
    ) -> Self {
        Self {
            log: oxplow_db::SqliteEventLogStore::new(db.clone(), vocabulary.clone()),
            db,
            vocabulary,
            project_dir,
            status_order: Arc::new(tokio::sync::Mutex::new(())),
            events,
            pump: None,
            turn_snapshots: None,
            harnesses,
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
        let vocabulary = self.vocabulary.clone();
        let project_dir = self.project_dir.clone();
        let harnesses = self.harnesses.clone();
        let applied = self
            .db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, "hook_ingest");
                record_tx(tx, &ev, &project_dir, &harnesses, thread, &env, now)
            })
            .await?;

        // The activity log and the Work panel's live turn rows re-read
        // `v_event` / `v_agent_turn` on the commit's `ModelsChanged`.
        outcome.closed_turn = applied.closed_turn;
        outcome.harness = applied.harness.clone();
        match applied.status {
            Some((state, detail)) => self.announce(thread, applied.session, state, detail).await,
            None => {
                self.announce_derived_status(&thread, applied.session, kind)
                    .await
            }
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

    /// Log and announce an agent session's status outside a hook (the ACP
    /// session's permission cards). `session` resolves as a hook's does.
    pub async fn set_status(
        &self,
        thread: &ThreadId,
        session: Option<oxplow_domain::AgentSessionId>,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<(), HookIngestError> {
        let _order = self.status_order.lock().await;
        let vocabulary = self.vocabulary.clone();
        let (thread_c, detail_c) = (*thread, detail.clone());
        let slot = self
            .db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, "hook_ingest");
                let slot = oxplow_db::agent_session_store::resolve_tx(tx, thread_c, session, None)?
                    .map(|s| s.id);
                let current = last_status_tx(tx, &vocabulary, thread_c, slot)?;
                if changed(current.as_ref(), state, detail_c.as_deref()) {
                    log_status_tx(tx, &ev, thread_c, slot, None, state, detail_c.clone())?;
                }
                Ok(slot)
            })
            .await?;
        self.announce(*thread, slot, state, detail).await;
        Ok(())
    }

    /// Tell the renderer `session`'s status on `thread`, with the thread's
    /// stream. A thread that's gone has nothing to show.
    async fn announce(
        &self,
        thread: ThreadId,
        session: Option<oxplow_domain::AgentSessionId>,
        state: AgentStatusState,
        detail: Option<String>,
    ) {
        let stream = self
            .db
            .read(move |conn| {
                oxplow_db::thread_store::get_tx(conn, thread)
                    .map(|t| t.map(|t| t.stream_id))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await;
        let Ok(Some(stream_id)) = stream else {
            return;
        };
        self.events.emit(OxplowEvent::AgentStatusChanged {
            thread_id: thread,
            stream_id,
            agent_session_id: session,
            state,
            detail,
        });
    }

    /// Tool hooks set no status of their own, but they change what the
    /// renderer derives (an open `Task` keeps a session working). Re-derive
    /// from the session's logged activity and announce it, keeping a
    /// status that parked it on the person.
    async fn announce_derived_status(
        &self,
        thread: &ThreadId,
        session: Option<oxplow_domain::AgentSessionId>,
        kind: HookKind,
    ) {
        if !matches!(kind, HookKind::PreToolUse | HookKind::PostToolUse) {
            return;
        }
        let current = {
            use oxplow_domain::stores::AgentStatusStore as _;
            oxplow_db::SqliteAgentStatusStore::new(self.db.clone(), self.vocabulary.clone())
                .get(thread, session)
                .await
                .ok()
                .flatten()
        };
        let (state, detail) = match current {
            Some(s) if s.state == AgentStatusState::AwaitingUser => {
                (AgentStatusState::AwaitingUser, s.detail)
            }
            _ => {
                let recent =
                    crate::agent_status_derive::recent_activity(&self.log, *thread, session)
                        .await
                        .unwrap_or_default();
                let derived =
                    crate::agent_status_derive::derive_session_status(&recent, Timestamp::now());
                (derived, None)
            }
        };
        self.announce(*thread, session, state, detail).await;
    }
}

/// Whether moving to `state`/`detail` is a change from `current`.
fn changed(current: Option<&AgentStatus>, state: AgentStatusState, detail: Option<&str>) -> bool {
    match current {
        Some(c) => c.state != state || c.detail.as_deref() != detail,
        None => true,
    }
}

/// What the ingest needs of the thread and the agent session a hook came
/// from.
struct ThreadRow {
    /// The session it came from (`agent_session_store::resolve_tx`); `None`
    /// for an agent oxplow didn't start.
    session: Option<oxplow_domain::AgentSessionId>,
    resume_session_id: String,
    /// The session's harness key; `None` with no session.
    harness: Option<String>,
    worktree: PathBuf,
}

/// The thread's worktree and the session `env` came from.
fn session_row_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    env: &HookEnvelope,
    project_dir: &Path,
) -> Result<Option<ThreadRow>, DomainError> {
    use rusqlite::OptionalExtension as _;
    let Some(worktree) = conn
        .query_row(
            "SELECT COALESCE(s.worktree_path, '')
               FROM threads th LEFT JOIN streams s ON s.id = th.stream_id
              WHERE th.id = ?1",
            [thread.value()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(oxplow_db::map_sql_err)?
    else {
        return Ok(None);
    };
    let session = oxplow_db::agent_session_store::resolve_tx(
        conn,
        thread,
        env.agent_session_id,
        env.session_id.as_deref(),
    )?;
    Ok(Some(ThreadRow {
        session: session.as_ref().map(|s| s.id),
        resume_session_id: session
            .as_ref()
            .map(|s| s.resume_session_id.clone())
            .unwrap_or_default(),
        harness: session.map(|s| s.harness),
        worktree: if worktree.is_empty() {
            project_dir.to_path_buf()
        } else {
            PathBuf::from(worktree)
        },
    }))
}

/// The envelope's state changes and events, in one transaction.
fn record_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    project_dir: &Path,
    harnesses: &HarnessRegistry,
    thread: ThreadId,
    env: &HookEnvelope,
    now: Timestamp,
) -> Result<Applied, DomainError> {
    let Some(row) = session_row_tx(conn, thread, env, project_dir)? else {
        return Ok(Applied::default()); // an unknown thread: the hook log only
    };
    let body: serde_json::Value = serde_json::from_str(&env.payload_json).unwrap_or_default();
    let session = env.session_id.as_deref().filter(|s| !s.is_empty());
    let slot = row.session;
    let mut applied = Applied {
        session: slot,
        harness: row.harness.clone(),
        ..Applied::default()
    };
    let mut status = None;
    let starts = starts_session(env.kind, &body);
    let transcript_path = body.get("transcript_path").and_then(|p| p.as_str());
    if starts {
        // The process that owned any open turn is gone: they end
        // interrupted, before the session start that resets the thread.
        let end = TurnEnd {
            answer: Some("session restarted"),
            ..TurnEnd::new(now, TurnOutcome::Interrupted)
        };
        // A resumed session's own turns keep its transcript (tsk924): it
        // is the file their tokens are in. Another session's file isn't.
        if let Some(sid) = session {
            let own = TurnEnd {
                transcript_path,
                ..end.clone()
            };
            for id in open_harness_turn_ids_tx(conn, thread, sid)? {
                if close_turn_tx(conn, ev, id, &own)?.is_some() && applied.closed_turn.is_none() {
                    applied.closed_turn = Some(id);
                }
            }
        }
        if let Some(id) = close_open_turns_tx(conn, ev, thread, slot, &end)? {
            applied.closed_turn = applied.closed_turn.or(Some(id));
        }
        status = Some((AgentStatusState::Idle, None));
    }
    if env.kind != HookKind::SessionEnd {
        if let Some(sid) = session {
            track_session_tx(conn, ev, thread, &row, sid, starts, now)?;
        }
    }
    applied.turn = open_turn_ids_in_tx(conn, thread, slot)?.first().copied();
    match env.kind {
        HookKind::UserPromptSubmit => {
            let reprompt = applied.turn.is_some();
            if !reprompt {
                let prompt = env.prompt.as_deref().unwrap_or_default();
                applied.turn = Some(open_turn_tx(conn, ev, thread, slot, prompt, session, now)?);
                applied.opened_turn = true;
            }
            let text = env.prompt.as_deref().filter(|p| !p.is_empty());
            let payload = AgentPromptSubmittedV1 {
                thread: thread_ref(thread),
                turn: applied.turn.map(turn_ref),
                session: session.map(str::to_string),
                reprompt,
                prompt: text
                    .map(|t| event_content_store::put_text_tx(conn, "agent", t))
                    .transpose()?,
            };
            let env = ev
                .typed::<AgentPromptSubmitted>(&payload)
                .with_anchors(activity_anchors_tx(conn, thread, slot)?)
                .with_subject([applied
                    .turn
                    .map(turn_ref)
                    .unwrap_or_else(|| thread_ref(thread))]);
            ev.append(conn, &env)?;
            status = Some((AgentStatusState::Running, None));
        }
        HookKind::PreToolUse | HookKind::PostToolUse => {
            // The call as its harness maps it: the sender's, else the
            // session's harness's reading of the body.
            let tool = env.tool.clone().or_else(|| {
                row.harness
                    .as_deref()
                    .and_then(|h| harnesses.get(h).ok())
                    .and_then(|h| h.tool_use(&body))
            });
            // A body that names no tool records nothing.
            if let Some(tool) = &tool {
                log_tool_tx(conn, ev, thread, &row, env, tool, &body, session)?;
            }
            let asking = tool.filter(|t| t.kind.waits_on_person());
            let asks = asking.is_some();
            if let (HookKind::PreToolUse, Some(tool)) = (env.kind, &asking) {
                // A question or a plan put to the person waits on them.
                status = Some((AgentStatusState::AwaitingUser, Some(asked_of(tool))));
            } else if env.kind == HookKind::PostToolUse
                && (asks
                    || last_status_tx(conn, ev.vocabulary, thread, slot)?
                        .is_some_and(|s| s.state == AgentStatusState::AwaitingUser))
            {
                // Answered, or the tool it was waiting on permission for ran.
                status = Some((AgentStatusState::Running, None));
            }
        }
        HookKind::Notification => {
            // A permission prompt (or a form to fill) waits on the person;
            // an idle reminder says nothing new.
            let waits = matches!(
                body.get("notification_type").and_then(|t| t.as_str()),
                Some("permission_prompt" | "elicitation_dialog")
            );
            if waits {
                let message = body
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Waiting on you")
                    .to_string();
                status = Some((AgentStatusState::AwaitingUser, Some(message)));
            }
        }
        HookKind::Stop | HookKind::Interrupt => {
            let (answer, outcome) = if env.kind == HookKind::Stop {
                let said = body
                    .get(LAST_ASSISTANT_MESSAGE)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty());
                (said, TurnOutcome::Completed)
            } else {
                (Some("interrupted"), TurnOutcome::Interrupted)
            };
            let end = TurnEnd {
                answer,
                transcript_path,
                // Counts a harness reported with the turn itself (ACP).
                usage: body
                    .get(TURN_USAGE_KEY)
                    .and_then(|u| serde_json::from_value(u.clone()).ok()),
                ..TurnEnd::new(now, outcome)
            };
            applied.closed_turn = close_open_turns_tx(conn, ev, thread, slot, &end)?;
            applied.turn = None;
            status = Some(if env.kind == HookKind::Interrupt {
                (AgentStatusState::Stopped, Some("interrupt".to_string()))
            } else {
                stop_status(answer)
            });
        }
        HookKind::SessionEnd => {
            // A harness's own SessionEnd names its session; a process exit
            // (the PTY's, for a harness that posts none) names only the
            // agent session, and ends the harness session it resumes.
            let exited = session.is_none();
            if !exited || slot.is_some() {
                // A session that ends mid-turn (an exit with no Stop)
                // leaves no one to close its turn.
                // Its transcript, so the turn's tokens are its own.
                let end = TurnEnd {
                    answer: Some("session ended"),
                    transcript_path,
                    ..TurnEnd::new(now, TurnOutcome::Interrupted)
                };
                let open = match session {
                    Some(sid) => open_harness_turn_ids_tx(conn, thread, sid)?,
                    None => open_turn_ids_in_tx(conn, thread, slot)?,
                };
                for id in open {
                    if close_turn_tx(conn, ev, id, &end)?.is_some() && applied.closed_turn.is_none()
                    {
                        applied.closed_turn = Some(id);
                    }
                }
                if applied.closed_turn.is_some() {
                    status = Some((AgentStatusState::Stopped, Some("session ended".into())));
                }
                applied.turn = open_turn_ids_in_tx(conn, thread, slot)?.first().copied();
                let ended =
                    session.or(Some(row.resume_session_id.as_str()).filter(|s| !s.is_empty()));
                if let Some(sid) = ended {
                    // The harness said so itself before its process went.
                    if !(exited && already_ended_tx(conn, slot)?) {
                        end_session_tx(conn, ev, thread, &row, sid, &body, now)?;
                    }
                }
            }
        }
        HookKind::SessionStart => {} // handled above
    }
    if let Some((state, detail)) = &status {
        if changed(
            last_status_tx(conn, ev.vocabulary, thread, slot)?.as_ref(),
            *state,
            detail.as_deref(),
        ) {
            log_status_tx(
                conn,
                ev,
                thread,
                slot,
                applied.closed_turn,
                *state,
                detail.clone(),
            )?;
        }
    }
    applied.status = status;
    Ok(applied)
}

/// A Stop parks the thread on the person when the agent's final message
/// ends in a question — its last line is the detail — else it goes idle.
/// The next prompt moves it on.
fn stop_status(answer: Option<&str>) -> (AgentStatusState, Option<String>) {
    let asked = answer
        .map(str::trim)
        .filter(|a| a.ends_with('?'))
        .and_then(|a| a.lines().map(str::trim).rfind(|l| !l.is_empty()));
    match asked {
        Some(question) => (AgentStatusState::AwaitingUser, Some(question.to_string())),
        None => (AgentStatusState::Idle, None),
    }
}

/// What a call that waits on the person asks: its question, or that a
/// plan waits for approval.
fn asked_of(tool: &ToolUse) -> String {
    match (&tool.question, tool.kind) {
        (Some(q), _) => q.clone(),
        (None, ToolKind::Plan) => "A plan to approve".to_string(),
        _ => "A question".to_string(),
    }
}

/// Whether this hook is a harness process starting (or resuming) a
/// session: a `SessionStart`, except the one a context compaction posts
/// mid-turn (Claude's `source: "compact"` keeps the same process and turn).
fn starts_session(kind: HookKind, body: &serde_json::Value) -> bool {
    kind == HookKind::SessionStart && body.get("source").and_then(|s| s.as_str()) != Some("compact")
}

/// Close every open turn of agent session `session` on the thread; returns
/// the newest one closed, which owns the turn-end snapshot.
fn close_open_turns_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    session: Option<oxplow_domain::AgentSessionId>,
    end: &TurnEnd<'_>,
) -> Result<Option<AgentTurnId>, DomainError> {
    let mut newest = None;
    // Newest first.
    for id in open_turn_ids_in_tx(conn, thread, session)? {
        if close_turn_tx(conn, ev, id, end)?.is_some() && newest.is_none() {
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
        .typed::<AgentSessionStarted>(&AgentSessionStartedV2 {
            session: session.to_string(),
            thread: thread_ref(thread),
            harness: row.harness.clone().unwrap_or_default(),
            resumed: row.resume_session_id == session,
        })
        .with_anchors(activity_anchors_tx(conn, thread, row.session)?)
        .with_subject(session_subject(thread, row.session));
    let first = env
        .clone()
        .with_dedupe_key(format!("session:{session}:started"));
    if !append_unique_tx(conn, ev.vocabulary, &first)? && starts {
        ev.append(conn, &env)?;
    }
    if let Some(id) = row.session.filter(|_| row.resume_session_id != session) {
        oxplow_db::agent_session_store::set_resume_tx(conn, id, session, now)?;
    }
    Ok(())
}

/// What an `agent.session.*` event is about: its thread, and its agent
/// session when one claims it.
fn session_subject(thread: ThreadId, slot: Option<oxplow_domain::AgentSessionId>) -> Vec<String> {
    std::iter::once(thread_ref(thread))
        .chain(slot.map(oxplow_domain::refs::build::agent_session_ref))
        .collect()
}

/// Whether agent session `slot`'s last session event is an
/// `agent.session.ended`: its harness ended it before the process exited.
fn already_ended_tx(
    conn: &rusqlite::Connection,
    slot: Option<oxplow_domain::AgentSessionId>,
) -> Result<bool, DomainError> {
    use rusqlite::OptionalExtension as _;
    let last: Option<String> = conn
        .query_row(
            "SELECT type FROM event_log
              WHERE agent_session_id = ?1
                AND type IN ('agent.session.started', 'agent.session.ended')
              ORDER BY seq DESC LIMIT 1",
            [slot.map(|s| s.value())],
            |r| r.get(0),
        )
        .optional()
        .map_err(oxplow_db::map_sql_err)?;
    Ok(last.as_deref() == Some("agent.session.ended"))
}

/// `SessionEnd`: log it, and drop the resume id only when an explicit
/// `/clear` ended exactly the session it points at — a normal exit keeps
/// it (a restart should resume), and clearing a stale session must not
/// wipe a newer one. (Until a hook names the post-clear session, a restart
/// would otherwise resurrect the cleared one.)
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
        .with_anchors(activity_anchors_tx(conn, thread, row.session)?)
        .with_subject(session_subject(thread, row.session));
    ev.append(conn, &env)?;
    if let Some(id) = row.session.filter(|_| reason == Some("clear")) {
        oxplow_db::agent_session_store::forget_resume_tx(conn, id, session, now)?;
    }
    Ok(())
}

/// `agent.tool.requested` / `agent.tool.finished` for a tool hook, with
/// the input (and output) stored by hash.
#[allow(
    clippy::too_many_arguments,
    reason = "the hook, its call and where it lands, read by one writer"
)]
fn log_tool_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    row: &ThreadRow,
    env: &HookEnvelope,
    tool: &ToolUse,
    body: &serde_json::Value,
    session: Option<&str>,
) -> Result<(), DomainError> {
    let recorded = crate::tool_calls::recorded(tool, &row.worktree);
    let content = |key: &str| -> Result<Option<ContentRef>, DomainError> {
        match body.get(key) {
            Some(v) if !v.is_null() => event_content_store::put_json_tx(conn, "agent", v).map(Some),
            _ => Ok(None),
        }
    };
    let dedupe = |phase: &str| {
        tool.call_id
            .as_deref()
            .map(|id| format!("{}:{id}:{phase}", session.unwrap_or("-")))
    };
    let anchors = activity_anchors_tx(conn, thread, row.session)?;
    let subject = anchors
        .turn_id
        .map(|t| turn_ref(AgentTurnId::new(t)))
        .unwrap_or_else(|| thread_ref(thread));
    let envelope = if env.kind == HookKind::PreToolUse {
        let decision = env.decision.clone().unwrap_or(ToolDecision {
            allowed: true,
            reason: None,
        });
        ev.typed::<AgentToolRequested>(&AgentToolRequestedV2 {
            tool: tool.name.clone(),
            kind: tool.kind,
            paths: recorded.paths,
            detail: recorded.detail,
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
        let finished = ev
            .typed::<AgentToolFinished>(&AgentToolFinishedV2 {
                tool: tool.name.clone(),
                kind: tool.kind,
                paths: recorded.paths,
                detail: recorded.detail,
                command: tool.command.clone(),
                ok: tool.ok,
                exit_code: tool.exit_code,
                input: content("tool_input")?,
                output: content("tool_response")?,
            })
            .with_dedupe_key_opt(dedupe("finished"));
        // Caused by the call's start, so what reads the run knows when it
        // began (tsk888: its reports are written after).
        let requested = match dedupe("requested") {
            Some(key) => oxplow_db::event_log_store::id_by_dedupe_tx(conn, &key)?,
            None => None,
        };
        match requested {
            Some(id) => finished.with_cause(id),
            None => finished,
        }
    };
    let envelope = envelope.with_anchors(anchors).with_subject([subject]);
    append_unique_tx(conn, ev.vocabulary, &envelope)?;
    Ok(())
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
            host: oxplow_domain::HostId::LOCAL,
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
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        db.transaction(move |tx| {
            oxplow_db::agent_session_store::insert_tx(
                tx,
                &oxplow_domain::agent_session::NewAgentSession::terminal(t.id, "claude"),
                now,
            )
        })
        .await
        .unwrap();
        let svc = HookIngestService::new(
            db,
            oxplow_domain::vocabulary::VocabularyHandle::core(),
            std::path::PathBuf::from("/p"),
            EventBus::new(),
            claude_registry(),
        );
        (svc, t.id)
    }

    /// The harnesses the fixture's sessions run: Claude Code, whose hook
    /// bodies these tests post.
    fn claude_registry() -> HarnessRegistry {
        let registry = HarnessRegistry::new(std::sync::Arc::new(String::new));
        registry.register(
            oxplow_harnesses::built_in("oxplow:claude-code", "claude", "Claude")
                .expect("the built-in"),
        );
        registry
    }

    /// A new service over the same database: a restarted daemon.
    fn restarted(svc: &HookIngestService) -> HookIngestService {
        HookIngestService::new(
            svc.db.clone(),
            svc.vocabulary.clone(),
            svc.project_dir.clone(),
            EventBus::new(),
            svc.harnesses.clone(),
        )
    }

    #[tokio::test]
    async fn set_status_logs_once_and_refreshes_the_activity_log() {
        let (svc, tid) = fixture().await;
        for _ in 0..2 {
            svc.set_status(
                &tid,
                Some(first_session()),
                AgentStatusState::AwaitingUser,
                Some("A?".into()),
            )
            .await
            .unwrap();
        }
        // The second call changed nothing: logged once.
        let events = logged(&svc).await;
        assert_eq!(of_type(&events, "agent.status.changed").len(), 1);
    }

    /// The fixture session's status as a freshly started daemon would
    /// read it.
    async fn status(svc: &HookIngestService, tid: ThreadId) -> Option<AgentStatus> {
        use oxplow_domain::stores::AgentStatusStore as _;
        oxplow_db::SqliteAgentStatusStore::new(svc.db.clone(), svc.vocabulary.clone())
            .get(&tid, Some(first_session()))
            .await
            .unwrap()
    }

    fn turns(svc: &HookIngestService) -> oxplow_db::SqliteAgentTurnStore {
        oxplow_db::SqliteAgentTurnStore::new(svc.db.clone())
    }

    /// Every event in the log, oldest first.
    async fn logged(svc: &HookIngestService) -> Vec<oxplow_domain::StoredEvent> {
        oxplow_db::SqliteEventLogStore::new(svc.db.clone(), svc.vocabulary.clone())
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
            agent_session_id: None,
            session_id: session.map(str::to_string),
            payload_json: body.to_string(),
            prompt: body
                .get("prompt")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            decision: None,
            tool: None,
        }
    }

    /// The session the fixture opens with its thread.
    fn first_session() -> oxplow_domain::AgentSessionId {
        oxplow_domain::AgentSessionId::new(1)
    }

    /// A hook from agent session `ses` (`X-Oxplow-Session`).
    fn hook_in(
        kind: HookKind,
        tid: ThreadId,
        ses: oxplow_domain::AgentSessionId,
        session: Option<&str>,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            agent_session_id: Some(ses),
            ..hook(kind, tid, session, body)
        }
    }

    /// Open another session on `thread`, after the ones it has.
    async fn open_session(
        svc: &HookIngestService,
        thread: ThreadId,
    ) -> oxplow_domain::AgentSessionId {
        svc.db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::insert_tx(
                    tx,
                    &oxplow_domain::agent_session::NewAgentSession::terminal(thread, "claude"),
                    Timestamp::now(),
                )
            })
            .await
            .unwrap()
            .id
    }

    /// The agent session each `agent.prompt.submitted` was anchored to.
    async fn prompt_sessions(
        svc: &HookIngestService,
    ) -> Vec<Option<oxplow_domain::AgentSessionId>> {
        of_type(&logged(svc).await, "agent.prompt.submitted")
            .iter()
            .map(|e| e.envelope.anchors.agent_session_id)
            .collect()
    }

    /// The open turns and the session each runs in.
    async fn open_turns(
        svc: &HookIngestService,
        tid: ThreadId,
    ) -> Vec<Option<oxplow_domain::AgentSessionId>> {
        let mut open: Vec<_> = turns(svc)
            .list_open(&tid)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.agent_session_id)
            .collect();
        open.sort_by_key(|s| s.map(|s| s.value()));
        open
    }

    /// Two sessions in one thread run their own turns: a Stop in one
    /// closes only its turn.
    #[tokio::test]
    async fn a_stop_closes_only_its_own_sessions_turn() {
        let (svc, tid) = fixture().await;
        let (a, b) = (first_session(), open_session(&svc, tid).await);
        for (ses, sid) in [(a, "ha"), (b, "hb")] {
            svc.ingest(hook_in(
                HookKind::UserPromptSubmit,
                tid,
                ses,
                Some(sid),
                json!({"prompt": "go"}),
            ))
            .await
            .unwrap();
        }
        assert_eq!(open_turns(&svc, tid).await, vec![Some(a), Some(b)]);
        svc.ingest(hook_in(HookKind::Stop, tid, a, Some("ha"), json!({})))
            .await
            .unwrap();
        assert_eq!(open_turns(&svc, tid).await, vec![Some(b)]);
    }

    /// A process starting in one session interrupts that session's turn,
    /// not the other's.
    #[tokio::test]
    async fn a_session_start_leaves_another_sessions_turn_open() {
        let (svc, tid) = fixture().await;
        let (a, b) = (first_session(), open_session(&svc, tid).await);
        for (ses, sid) in [(a, "ha"), (b, "hb")] {
            svc.ingest(hook_in(
                HookKind::UserPromptSubmit,
                tid,
                ses,
                Some(sid),
                json!({"prompt": "go"}),
            ))
            .await
            .unwrap();
        }
        svc.ingest(hook_in(
            HookKind::SessionStart,
            tid,
            a,
            Some("ha2"),
            json!({"source": "startup"}),
        ))
        .await
        .unwrap();
        assert_eq!(open_turns(&svc, tid).await, vec![Some(b)]);
    }

    /// Which session a hook came from: the header names it; else the
    /// harness's session id finds the session it resumes; else the thread's
    /// newest open session. A header naming another thread's session is
    /// ignored.
    #[tokio::test]
    async fn the_header_beats_the_harness_id_which_beats_the_newest_session() {
        let (svc, tid) = fixture().await;
        let (a, b) = (first_session(), open_session(&svc, tid).await);
        let prompt = |sid: Option<&str>| {
            hook(
                HookKind::UserPromptSubmit,
                tid,
                sid,
                json!({"prompt": "go"}),
            )
        };
        // A's first hook makes "ha" its resume id.
        svc.ingest(HookEnvelope {
            agent_session_id: Some(a),
            ..prompt(Some("ha"))
        })
        .await
        .unwrap();
        // No header: "ha" is A's, though B is newer.
        svc.ingest(prompt(Some("ha"))).await.unwrap();
        // The header wins over the harness id (and B now resumes "ha").
        svc.ingest(HookEnvelope {
            agent_session_id: Some(b),
            ..prompt(Some("ha"))
        })
        .await
        .unwrap();
        // Neither: the newest open session.
        svc.ingest(prompt(None)).await.unwrap();
        // Another thread's session in the header: ignored, so the newest
        // open session again.
        let other = svc
            .db
            .transaction(|tx| {
                tx.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (2, 1, 'y', 'queued', 't', 't')",
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .map(|_| ThreadId::new(2))
            .unwrap();
        let foreign = open_session(&svc, other).await;
        svc.ingest(HookEnvelope {
            agent_session_id: Some(foreign),
            ..prompt(None)
        })
        .await
        .unwrap();
        assert_eq!(
            prompt_sessions(&svc).await,
            vec![Some(a), Some(a), Some(b), Some(b), Some(b)]
        );
    }

    /// A process exit (the PTY's SessionEnd, naming only the agent
    /// session) closes that session's turn and ends the harness session it
    /// resumes — once: not again when the harness already ended it.
    #[tokio::test]
    async fn a_process_exit_ends_its_session_once() {
        let (svc, tid) = fixture().await;
        let a = first_session();
        let exit = || HookEnvelope {
            agent_session_id: Some(a),
            ..hook(HookKind::SessionEnd, tid, None, json!({"reason": "exit"}))
        };
        svc.ingest(hook_in(
            HookKind::UserPromptSubmit,
            tid,
            a,
            Some("h1"),
            json!({"prompt": "go"}),
        ))
        .await
        .unwrap();
        svc.ingest(exit()).await.unwrap();
        assert!(open_turns(&svc, tid).await.is_empty());
        let events = logged(&svc).await;
        let ended = of_type(&events, "agent.session.ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].envelope.payload["session"], "h1");
        assert_eq!(ended[0].envelope.payload["reason"], "exit");
        assert_eq!(ended[0].envelope.anchors.agent_session_id, Some(a));
        assert!(ended[0]
            .envelope
            .subject
            .contains(&oxplow_domain::refs::build::agent_session_ref(a)));

        // The harness ends it itself, then its process exits: one more.
        svc.ingest(hook_in(
            HookKind::SessionStart,
            tid,
            a,
            Some("h1"),
            json!({"source": "resume"}),
        ))
        .await
        .unwrap();
        svc.ingest(hook_in(
            HookKind::SessionEnd,
            tid,
            a,
            Some("h1"),
            json!({"reason": "other"}),
        ))
        .await
        .unwrap();
        svc.ingest(exit()).await.unwrap();
        assert_eq!(of_type(&logged(&svc).await, "agent.session.ended").len(), 2);
    }

    /// A hook on a thread with no session (an agent oxplow didn't start)
    /// records with the thread's anchors and no session.
    #[tokio::test]
    async fn a_hook_with_no_session_records_none() {
        let (svc, tid) = fixture().await;
        svc.db
            .transaction(|tx| {
                tx.execute_batch("DELETE FROM agent_session")
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("x"),
            json!({"prompt": "go"}),
        ))
        .await
        .unwrap();
        assert_eq!(prompt_sessions(&svc).await, vec![None]);
        assert_eq!(open_turns(&svc, tid).await, vec![None]);
    }

    async fn open_effort(svc: &HookIngestService) -> i64 {
        svc.db
            .transaction(|c| {
                c.execute(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                       VALUES ('work_item:issues:ENG-1', 1, '2026-01-01T00:00:00.000000Z')",
                    [],
                )
                .map_err(|e| DomainError::Storage(e.to_string()))?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// P3.3: a finished tool call is an `agent.tool.finished` anchored to
    /// stream, thread, agent session, turn and effort, with its input and
    /// output stored by hash and its path made worktree-relative.
    #[tokio::test]
    async fn a_finished_tool_is_logged_with_five_anchors_and_its_content() {
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
        assert_eq!(e.anchors.agent_session_id, Some(first_session()));
        assert_eq!(e.payload["tool"], "Edit");
        assert_eq!(e.payload["kind"], "edit");
        assert_eq!(e.payload["paths"], json!(["src/a.rs"]));
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

    /// tsk888: a call's `agent.tool.finished` is caused by its
    /// `agent.tool.requested` (one tool use id), so what reads the run
    /// knows when it started.
    #[tokio::test]
    async fn a_finished_tool_call_is_caused_by_its_request() {
        let (svc, tid) = fixture().await;
        for kind in [HookKind::PreToolUse, HookKind::PostToolUse] {
            svc.ingest(hook(
                kind,
                tid,
                Some("s1"),
                json!({"tool_name": "Bash", "tool_use_id": "tu9", "tool_input": {"command": "cargo test"}}),
            ))
            .await
            .unwrap();
        }
        let events = logged(&svc).await;
        let requested = of_type(&events, "agent.tool.requested");
        let finished = of_type(&events, "agent.tool.finished");
        assert_eq!(
            finished[0].envelope.cause.as_ref(),
            Some(&requested[0].envelope.id)
        );
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
                    "SELECT resume_session_id FROM agent_session WHERE thread_id = 1",
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

    /// tsk449: a session that ends without a Stop (an exit mid-turn) closes
    /// its own open turn — interrupted, "session ended" — so the turn
    /// doesn't hold the quiet-period trigger open forever. Another
    /// session's end leaves it alone.
    #[tokio::test]
    async fn a_session_ending_mid_turn_closes_its_turn() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(
            HookKind::UserPromptSubmit,
            tid,
            Some("s1"),
            json!({"prompt": "go"}),
        ))
        .await
        .unwrap();
        let open = |svc: &HookIngestService| {
            let db = svc.db.clone();
            async move {
                db.read(move |c| oxplow_db::agent_stores::open_turn_ids_tx(c, tid))
                    .await
                    .unwrap()
            }
        };
        assert_eq!(open(&svc).await.len(), 1);
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("other"),
            json!({"reason": "other"}),
        ))
        .await
        .unwrap();
        assert_eq!(open(&svc).await.len(), 1, "another session's end");
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("s1"),
            json!({"reason": "prompt_input_exit"}),
        ))
        .await
        .unwrap();
        assert!(open(&svc).await.is_empty());
        let events = logged(&svc).await;
        let ended = of_type(&events, "agent.turn.ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].envelope.payload["outcome"], "interrupted");
        let status = of_type(&events, "agent.status.changed");
        assert_eq!(status.last().unwrap().envelope.payload["state"], "stopped");
    }

    /// tsk924: a turn a session end or a restart closes keeps its
    /// transcript, so its tokens are recorded as its own — the session's
    /// own file, for a resume of that session; another session's file is
    /// someone else's.
    #[tokio::test]
    async fn a_turn_closed_by_its_session_keeps_its_transcript() {
        let (svc, tid) = fixture().await;
        let prompt = |sid: &'static str| {
            hook(
                HookKind::UserPromptSubmit,
                tid,
                Some(sid),
                json!({"prompt": "go"}),
            )
        };
        svc.ingest(prompt("s1")).await.unwrap();
        svc.ingest(hook(
            HookKind::SessionEnd,
            tid,
            Some("s1"),
            json!({"reason": "prompt_input_exit", "transcript_path": "/s1.jsonl"}),
        ))
        .await
        .unwrap();
        // A resume of s1 closes its open turn with s1's file…
        svc.ingest(prompt("s1")).await.unwrap();
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s1"),
            json!({"source": "resume", "transcript_path": "/s1.jsonl"}),
        ))
        .await
        .unwrap();
        // …a new session's start closes s1's with none of its own.
        svc.ingest(prompt("s1")).await.unwrap();
        svc.ingest(hook(
            HookKind::SessionStart,
            tid,
            Some("s2"),
            json!({"source": "startup", "transcript_path": "/s2.jsonl"}),
        ))
        .await
        .unwrap();
        let events = logged(&svc).await;
        let paths: Vec<serde_json::Value> = of_type(&events, "agent.turn.ended")
            .iter()
            .map(|e| e.envelope.payload["transcript_path"].clone())
            .collect();
        assert_eq!(
            paths,
            vec![
                json!("/s1.jsonl"),
                json!("/s1.jsonl"),
                serde_json::Value::Null
            ]
        );
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
    async fn a_prompt_opens_a_turn_and_a_stop_closes_it() {
        // The Work panel's live turn rows re-read `v_agent_turn`.
        let (svc, tid) = fixture().await;
        let open = |svc: &HookIngestService| {
            let db = svc.db.clone();
            async move {
                db.read(|c| {
                    c.query_row(
                        "SELECT count(*) FROM agent_turn WHERE ended_at IS NULL",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
            }
        };
        let envelope = |kind, prompt: Option<&str>| HookEnvelope {
            kind,
            thread_id: Some(tid),
            stream_id: None,
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: prompt.map(str::to_string),
            decision: None,
            tool: None,
        };
        svc.ingest(envelope(HookKind::UserPromptSubmit, Some("p")))
            .await
            .unwrap();
        assert_eq!(open(&svc).await, 1);
        let closed = svc.ingest(envelope(HookKind::Stop, None)).await.unwrap();
        assert!(closed.closed_turn.is_some());
        assert_eq!(open(&svc).await, 0);
        // A Stop with nothing open closes nothing.
        let again = svc.ingest(envelope(HookKind::Stop, None)).await.unwrap();
        assert!(again.closed_turn.is_none());
    }

    #[tokio::test]
    async fn user_prompt_opens_turn_and_marks_running() {
        let (svc, tid) = fixture().await;
        let env = HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("sess".into()),
            payload_json: "{}".into(),
            prompt: Some("do the thing".into()),
            decision: None,
            tool: None,
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
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("do".into()),
            decision: None,
            tool: None,
        };
        svc.ingest(prompt_env).await.unwrap();
        let stop = HookEnvelope {
            kind: HookKind::Stop,
            thread_id: Some(tid),
            stream_id: None,
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
            tool: None,
        };
        svc.ingest(stop).await.unwrap();
        assert!(turns(&svc).list_open(&tid).await.unwrap().is_empty());
        let status = status(&svc, tid).await.unwrap();
        assert_eq!(status.state, AgentStatusState::Idle);
    }

    #[tokio::test]
    async fn interrupt_closes_open_turn() {
        let (svc, tid) = fixture().await;
        svc.ingest(HookEnvelope {
            kind: HookKind::UserPromptSubmit,
            thread_id: Some(tid),
            stream_id: None,
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("p".into()),
            decision: None,
            tool: None,
        })
        .await
        .unwrap();
        svc.ingest(HookEnvelope {
            kind: HookKind::Interrupt,
            thread_id: Some(tid),
            stream_id: None,
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
            tool: None,
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
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: None,
            decision: None,
            tool: None,
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
            agent_session_id: None,
            session_id: None,
            payload_json: "{}".into(),
            prompt: Some("orphan".into()),
            decision: None,
            tool: None,
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
                agent_session_id: None,
                session_id: None,
                payload_json: "{}".into(),
                prompt: Some(prompt.into()),
                decision: None,
                tool: None,
            })
            .await
            .unwrap();
        }
        let open = turns(&svc).list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].prompt, "first");
    }

    /// The derived status the rail would show for the fixture session now.
    async fn derived(svc: &HookIngestService, tid: ThreadId) -> AgentStatusState {
        let recent =
            crate::agent_status_derive::recent_activity(&svc.log, tid, Some(first_session()))
                .await
                .unwrap();
        crate::agent_status_derive::derive_session_status(&recent, Timestamp::now())
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

    /// A turn whose final message ends in a question waits on the person,
    /// the question as the detail, across a restart; the next prompt
    /// moves it on.
    #[tokio::test]
    async fn a_final_question_waits_on_the_person() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(HookKind::UserPromptSubmit, tid, None, json!({})))
            .await
            .unwrap();
        svc.ingest(hook(
            HookKind::Stop,
            tid,
            None,
            json!({ LAST_ASSISTANT_MESSAGE: "Done with the parser.\n\nShould I also fix the lexer?" }),
        ))
        .await
        .unwrap();
        let svc = restarted(&svc);
        let s = status(&svc, tid).await.unwrap();
        assert_eq!(s.state, AgentStatusState::AwaitingUser);
        assert_eq!(s.detail.as_deref(), Some("Should I also fix the lexer?"));
        svc.ingest(hook(HookKind::UserPromptSubmit, tid, None, json!({})))
            .await
            .unwrap();
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Running
        );
        svc.ingest(hook(
            HookKind::Stop,
            tid,
            None,
            json!({ LAST_ASSISTANT_MESSAGE: "All done." }),
        ))
        .await
        .unwrap();
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Idle
        );
    }

    /// A question or plan the agent puts to the person (AskUserQuestion,
    /// ExitPlanMode) waits on them until it's answered.
    #[tokio::test]
    async fn a_pending_question_tool_waits_until_answered() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(HookKind::UserPromptSubmit, tid, None, json!({})))
            .await
            .unwrap();
        svc.ingest(hook(
            HookKind::PreToolUse,
            tid,
            None,
            json!({ "tool_name": "AskUserQuestion",
                    "tool_input": { "questions": [{ "question": "Which store?" }] } }),
        ))
        .await
        .unwrap();
        let s = status(&svc, tid).await.unwrap();
        assert_eq!(s.state, AgentStatusState::AwaitingUser);
        assert_eq!(s.detail.as_deref(), Some("Which store?"));
        svc.ingest(hook(
            HookKind::PostToolUse,
            tid,
            None,
            json!({ "tool_name": "AskUserQuestion" }),
        ))
        .await
        .unwrap();
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Running
        );
    }

    /// A permission prompt (Claude's Notification hook) waits on the
    /// person until the tool runs; an idle reminder doesn't.
    #[tokio::test]
    async fn a_permission_prompt_waits_until_the_tool_runs() {
        let (svc, tid) = fixture().await;
        svc.ingest(hook(HookKind::UserPromptSubmit, tid, None, json!({})))
            .await
            .unwrap();
        svc.ingest(hook(
            HookKind::Notification,
            tid,
            None,
            json!({ "notification_type": "idle_prompt", "message": "Claude is waiting for your input" }),
        ))
        .await
        .unwrap();
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Running
        );
        svc.ingest(hook(
            HookKind::Notification,
            tid,
            None,
            json!({ "notification_type": "permission_prompt",
                    "message": "Claude needs your permission to use Bash" }),
        ))
        .await
        .unwrap();
        let s = status(&svc, tid).await.unwrap();
        assert_eq!(s.state, AgentStatusState::AwaitingUser);
        assert_eq!(
            s.detail.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        svc.ingest(hook(
            HookKind::PostToolUse,
            tid,
            None,
            json!({ "tool_name": "Bash" }),
        ))
        .await
        .unwrap();
        assert_eq!(
            status(&svc, tid).await.unwrap().state,
            AgentStatusState::Running
        );
    }
}
