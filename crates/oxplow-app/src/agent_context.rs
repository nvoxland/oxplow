//! The context oxplow hands an agent, shared by every transport (tsk334,
//! P3.8): the `<session-context>` block (deduped per session — it names
//! the other live agents in the worktree, so it changes as they come and
//! go), the open handoffs to the thread no agent was given yet, prompt
//! advisories and the open effort's decisions on a prompt; the
//! thread's undelivered nudges after a tool call. What an agent *did* is
//! recorded elsewhere — the hook ingest logs `agent.*` events and pump
//! reactors record from them (`tool_call_reactors`, `post_tool_reactors`,
//! the token reactor).
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block): the last context per agent session. The
//! session's resume id is the hook ingest's (`hook_ingest`). See
//! `.context/agent-model.md`.

use std::collections::HashMap;

use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::{AgentSessionId, HandoffId, StreamId, ThreadId};
use parking_lot::Mutex;
use tracing::warn;

use crate::agent_prompt::{build_session_context_block, OtherAgent, Surroundings};
use crate::Services;

/// The last context block per agent session.
#[derive(Default)]
struct ContextState {
    /// Last context block returned per session (and per `…#decisions`
    /// key). The launch prompt already carries this data, so a
    /// byte-identical repeat adds noise without new information.
    last_context_by_session_id: HashMap<String, String>,
}

/// How long a tool call's hook waits for the reactors that write nudges.
const POST_TOOL_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

/// Dedupe key suffix for the decisions block, beside the session-context
/// block's plain session-id key.
const DECISIONS_KEY_SUFFIX: &str = "#decisions";

/// Recording and context shared by every agent transport
/// (`Services.agent_context`).
#[derive(Default)]
pub struct AgentContext {
    state: Mutex<ContextState>,
}

impl AgentContext {
    /// A fresh agent context (startup / resume / clear / compact): forget
    /// the session's baselines so the next prompt carries fresh context.
    pub fn reset_session(&self, session_id: Option<&str>) {
        let Some(session_id) = session_id else {
            return;
        };
        let mut state = self.state.lock();
        state.last_context_by_session_id.remove(session_id);
        state
            .last_context_by_session_id
            .remove(&format!("{session_id}{DECISIONS_KEY_SUFFIX}"));
    }

    /// What the agent hears after a tool call: the thread's undelivered
    /// nudges (collection and post-tool advisories). The recording itself is the pump's
    /// (`tool_call_reactors`, `post_tool_reactors`, P3.5–P3.6): this waits
    /// up to [`POST_TOOL_SETTLE`] for the two reactors that write nudges,
    /// then takes whatever is undelivered — a nudge that lands after the
    /// window goes out on the thread's next tool call instead of being lost.
    /// Collection nudges come before advisories.
    /// Nothing for a session whose harness can't carry context after a
    /// tool call (`tool_context`): its nudges wait for a prompt.
    pub async fn post_tool_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        harness: Option<&str>,
    ) -> Option<String> {
        if !carries(svc, harness, TOOL_CONTEXT) {
            return None;
        }
        svc.event_pump
            .settle(
                &[
                    crate::post_tool_reactors::COLLECTION,
                    crate::post_tool_reactors::POST_TOOL_ADVISORIES,
                ],
                POST_TOOL_SETTLE,
            )
            .await;
        undelivered(svc, thread_id).await
    }

    /// Context for a human's prompt in agent session `agent_session` (its
    /// harness's own id is `session_id`), delivered by the hook answer
    /// that carries it: built ([`Self::build_context`]) and committed
    /// ([`Self::commit`]) in one step, the answer counting as the success.
    /// Nothing for a session whose harness can't carry context on a prompt
    /// (`prompt_context`): nothing meant for its agent is taken.
    pub async fn prompt_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        agent_session: Option<AgentSessionId>,
        harness: Option<&str>,
    ) -> Option<String> {
        let built = self
            .build_context(svc, thread_id, session_id, agent_session, harness)
            .await?;
        let text = built.text.clone();
        self.commit(svc, built).await;
        Some(text)
    }

    /// What a prompt in the session would carry, built and **not yet
    /// taken**: the `<session-context>` block (only when it differs from
    /// what the session was last sent), the thread's undelivered handoffs,
    /// nudges (prompt advisories, and the hints its last turn's end left)
    /// and the open effort's decisions (once per session, and when they
    /// change). Nothing changes until [`Self::commit`] — the delivery can
    /// fail, and then the next build carries it all again.
    pub async fn build_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        agent_session: Option<AgentSessionId>,
        harness: Option<&str>,
    ) -> Option<PendingContext> {
        if !carries(svc, harness, PROMPT_CONTEXT) {
            return None;
        }
        let mut pending = PendingContext {
            text: String::new(),
            baselines: Vec::new(),
            handoffs: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session,
        };
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id, agent_session)
            .await;
        if let (Some(key), Some(block)) = (session_id, &ctx_block) {
            pending.baselines.push((key.to_string(), block.clone()));
        }
        let handoffs_block = handoffs(svc, *thread_id).await;
        if let Some((_, ids)) = &handoffs_block {
            pending.handoffs = ids.clone();
        }
        let basket_block = context_basket(svc, *thread_id).await;
        if let Some((_, ids)) = &basket_block {
            pending.basket = ids.clone();
        }
        // Prompt advisories land as nudges beside any a turn's end or a
        // late tool call left; the prompt carries them all.
        crate::advisories::for_thread(
            &svc.advisory_deps(),
            thread_id,
            crate::extensions::AdvisoryOn::Prompt,
            None,
        )
        .await;
        let advisory_block = waiting_nudges(svc, thread_id).await;
        if let Some((_, ids)) = &advisory_block {
            pending.nudges = ids.clone();
        }
        let decisions_block = self
            .refreshed_decisions_context(svc, thread_id, session_id)
            .await;
        if let (Some(key), Some(block)) = (session_id, &decisions_block) {
            pending
                .baselines
                .push((format!("{key}{DECISIONS_KEY_SUFFIX}"), block.clone()));
        }
        let combined: String = [
            ctx_block,
            handoffs_block.map(|(text, _)| text),
            basket_block.map(|(text, _)| text),
            advisory_block.map(|(text, _)| text),
            decisions_block,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n");
        if combined.is_empty() {
            return None;
        }
        pending.text = combined;
        Some(pending)
    }

    /// The agent has what [`Self::build_context`] built: remember the
    /// blocks as sent, stamp the handoffs it carried delivered to the
    /// session (by id), and mark its nudges delivered.
    pub async fn commit(&self, svc: &Services, built: PendingContext) {
        {
            let mut state = self.state.lock();
            for (key, block) in built.baselines {
                state.last_context_by_session_id.insert(key, block);
            }
        }
        if !built.handoffs.is_empty() {
            if let Err(err) = svc
                .handoff_store
                .mark_delivered(built.handoffs, built.agent_session)
                .await
            {
                warn!(?err, "stamping the delivered handoffs failed");
            }
        }
        if !built.nudges.is_empty() {
            if let Err(err) = svc.nudge_store.mark_delivered(built.nudges).await {
                warn!(?err, "marking the delivered nudges failed");
            }
        }
        if !built.basket.is_empty() {
            let (ids, session) = (built.basket, built.agent_session.map(|s| s.value()));
            if let Err(err) = svc
                .db
                .transaction(move |tx| {
                    oxplow_db::thread_context_store::mark_delivered_tx(tx, &ids, session)
                })
                .await
            {
                warn!(?err, "stamping the delivered context basket failed");
            }
        }
    }

    /// A fresh `<session-context>` block, naming the other live agents in
    /// the worktree, unless it's unchanged for this session or the project
    /// turned injection off.
    async fn refreshed_session_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
        agent_session: Option<AgentSessionId>,
    ) -> Option<String> {
        let cfg = svc.config.read().ok()?.clone();
        if !cfg.inject_session_context {
            return None;
        }
        let thread = svc.thread_store.get(thread_id).await.ok().flatten()?;
        let stream = svc
            .stream_store
            .get(&thread.stream_id)
            .await
            .ok()
            .flatten()?;
        let around = surroundings(svc, stream.id, thread.id, agent_session).await;
        let block = build_session_context_block(&stream, Some(&thread), &around);
        self.would_emit(session_id, &block).then_some(block)
    }

    /// The open effort's recorded decisions, on the first prompt of a
    /// session and whenever they change.
    async fn refreshed_decisions_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Option<String> {
        use crate::EffortStore as _;
        let effort = svc
            .effort_store
            .find_open_for_thread(thread_id)
            .await
            .ok()
            .flatten()?;
        let block = crate::reasoning::effort_decisions_block(&svc.sql, effort.id.value()).await?;
        let key = session_id.map(|s| format!("{s}{DECISIONS_KEY_SUFFIX}"));
        self.would_emit(key.as_deref(), &block).then_some(block)
    }

    /// Whether `block` is new for `session_id`: it differs from what the
    /// session was last sent (recorded by [`Self::commit`]). Without a
    /// session id it always is: suppressing could hide a change from
    /// another session sharing the thread.
    fn would_emit(&self, session_id: Option<&str>, block: &str) -> bool {
        let Some(session_id) = session_id else {
            return true;
        };
        self.state
            .lock()
            .last_context_by_session_id
            .get(session_id)
            .is_none_or(|previous| previous != block)
    }
}

/// Context built for an agent and not yet delivered
/// ([`AgentContext::build_context`]): the text, and what committing it
/// ([`AgentContext::commit`]) records — the blocks as the session's
/// baselines, the handoffs and nudges it carried as delivered.
pub struct PendingContext {
    pub text: String,
    baselines: Vec<(String, String)>,
    handoffs: Vec<HandoffId>,
    nudges: Vec<i64>,
    /// The context basket's items it carries (`thread_context_item.id`).
    basket: Vec<i64>,
    agent_session: Option<AgentSessionId>,
}

impl PendingContext {
    /// A context that commits nothing: for a double standing in for a
    /// host in tests.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            baselines: Vec::new(),
            handoffs: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session: None,
        }
    }

    /// The handoffs it carries (by id), for tests.
    pub fn handoff_ids(&self) -> &[HandoffId] {
        &self.handoffs
    }
}

/// The agent-harness feature: context oxplow returns to a prompt hook
/// reaches the agent.
pub const PROMPT_CONTEXT: &str = "prompt_context";
/// The agent-harness feature: context oxplow returns to a tool hook
/// reaches the agent before its next step.
pub const TOOL_CONTEXT: &str = "tool_context";

/// Whether `harness` (a session's, by registry key) declares `channel`.
/// With no harness known (a hook from no session) the answer is sent.
pub fn carries(svc: &Services, harness: Option<&str>, channel: &str) -> bool {
    let Some(harness) = harness else {
        return true;
    };
    svc.capabilities
        .get("agent_harness", harness)
        .is_some_and(|h| h.has_feature(channel))
}

/// Characters of nudges one hook carries to the agent; the rest wait for
/// its next.
pub const NUDGE_BUDGET: usize = 2_000;

/// The agent's undelivered nudges on the thread as one block, up to
/// [`NUDGE_BUDGET`], marked delivered: each reaches the agent once, on
/// whichever hook comes first, oxplow's own before advisories.
async fn undelivered(svc: &Services, thread_id: &ThreadId) -> Option<String> {
    let nudges = match svc
        .nudge_store
        .take_for_agent(&thread_id.to_string(), NUDGE_BUDGET)
        .await
    {
        Ok(n) => n,
        Err(err) => {
            warn!(?err, "reading undelivered nudges failed");
            return None;
        }
    };
    let text: Vec<String> = nudges.into_iter().map(|n| n.message).collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// The agent's undelivered nudges, as [`undelivered`] would take them, and
/// their ids, still waiting: a prompt's delivery marks them
/// ([`AgentContext::commit`]) once the agent has the text.
async fn waiting_nudges(svc: &Services, thread_id: &ThreadId) -> Option<(String, Vec<i64>)> {
    let nudges = match svc
        .nudge_store
        .peek_for_agent(&thread_id.to_string(), NUDGE_BUDGET)
        .await
    {
        Ok(n) => n,
        Err(err) => {
            warn!(?err, "reading undelivered nudges failed");
            return None;
        }
    };
    let ids = nudges.iter().map(|n| n.id).collect();
    let text: Vec<String> = nudges.into_iter().map(|n| n.message).collect();
    (!text.is_empty()).then(|| (text.join("\n\n"), ids))
}

/// Characters of one piece of text from the context basket a prompt
/// carries; the rest is read through where it came from.
pub const BASKET_TEXT_IN_CONTEXT: usize = 4_000;

/// The thread's context basket — what the person collected for its
/// agents and no agent was given yet — as one `<context-basket>` block and
/// the items' ids, still undelivered: they reach the first prompt of any
/// agent in the thread, and [`AgentContext::commit`] stamps them once the
/// agent has the text, which empties the basket.
async fn context_basket(svc: &Services, thread: ThreadId) -> Option<(String, Vec<i64>)> {
    let items = match svc
        .db
        .read(move |c| oxplow_db::thread_context_store::undelivered_tx(c, thread))
        .await
    {
        Ok(items) if !items.is_empty() => items,
        Ok(_) => return None,
        Err(err) => {
            warn!(?err, "reading the thread's context basket failed");
            return None;
        }
    };
    let ids = items.iter().map(|i| i.id).collect();
    Some((basket_block(&items), ids))
}

/// The `<context-basket>` block for `items`: each page as a mention the
/// agent reads with `oxplow_read`, each piece of text quoted with where it
/// came from.
pub fn basket_block(items: &[oxplow_db::thread_context_store::ContextItem]) -> String {
    use oxplow_db::thread_context_store::ContextKind;
    let mut s = String::from(
        "<context-basket>\nThe person collected these for this prompt while exploring. \
         Read a `[oxplow uri …]` with `oxplow_read`.\n",
    );
    for item in items {
        let label = item
            .label
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        match (item.kind, &item.source_uri, &item.text) {
            (ContextKind::Page, Some(uri), _) => {
                s.push_str(&format!("- [oxplow uri {uri}]{label}\n"));
            }
            (_, source, Some(text)) => {
                let from = source
                    .as_deref()
                    .map(|u| format!(" from [oxplow uri {u}]"))
                    .unwrap_or_default();
                let mut text = text.clone();
                if text.len() > BASKET_TEXT_IN_CONTEXT {
                    let mut end = BASKET_TEXT_IN_CONTEXT;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                    text.push_str("\n… (cut; read the rest where it came from)");
                }
                let fence = if text.contains("```") { "````" } else { "```" };
                s.push_str(&format!("- Text{from}{label}:\n{fence}\n{text}\n{fence}\n"));
            }
            _ => {}
        }
    }
    s.push_str("</context-basket>");
    s
}

/// Characters of a handoff's body its prompt carries; the rest is read
/// through its ref.
pub const HANDOFF_BODY_IN_CONTEXT: usize = 1_500;

/// The open handoffs to `thread` no agent was given yet, as one
/// `<handoffs>` block and their ids, still undelivered: each reaches the
/// thread's agents once, at the first prompt that carries it — never with
/// a tool call, so it doesn't land in the middle of other work. The stamp
/// is [`AgentContext::commit`]'s, once the agent has the text.
async fn handoffs(svc: &Services, thread: ThreadId) -> Option<(String, Vec<HandoffId>)> {
    use oxplow_db::semantic_layer::SqlCell;
    let taken = match svc.handoff_store.peek_for_prompt(thread).await {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => return None,
        Err(err) => {
            warn!(?err, "reading the thread's handoffs failed");
            return None;
        }
    };
    let ids: Vec<HandoffId> = taken.iter().map(|h| h.id).collect();
    // How the person sees each: its sender's thread and session, its item.
    let labels: HashMap<String, (String, Option<String>, Option<String>)> = match svc
        .sql
        .query_sql(
            "SELECT h.uri, t.title, s.display_name, w.title
               FROM v_handoff h
               JOIN v_thread t ON t.id = h.from_thread_id
               LEFT JOIN v_agent_session s ON s.id = h.from_session_id
               LEFT JOIN v_work_item w ON w.uri = h.work_item
              WHERE h.to_thread_id = ?1",
            vec![SqlCell::Int(thread.value())],
            None,
        )
        .await
    {
        Ok(out) => {
            let text = |c: &SqlCell| match c {
                SqlCell::Text(t) => Some(t.clone()),
                _ => None,
            };
            out.rows
                .iter()
                .filter_map(|r| Some((text(&r[0])?, (text(&r[1])?, text(&r[2]), text(&r[3])))))
                .collect()
        }
        Err(err) => {
            warn!(?err, "reading the handoffs' senders failed");
            HashMap::new()
        }
    };
    let mut s = String::from(
        "<handoffs>\nWork handed to this thread. When you take one on or decline it, say so with \
         `oxplow.handoff.resolve { handoff, outcome: taken | declined, note? }`: taking one \
         moves its work item onto this thread, and its sender reads the note.\n",
    );
    for h in taken {
        let r = oxplow_domain::uri::build::handoff_uri(h.id);
        let (from_title, from_name, item_title) = labels.get(&r).cloned().unwrap_or_default();
        let by = from_name.unwrap_or_else(|| "the person".into());
        let item = item_title
            .map(|t| format!("; work item \"{t}\""))
            .unwrap_or_default();
        s.push_str(&format!(
            "\n- `{r}` from thread {from_title} (`{}`), by {by}, {}{item}:\n",
            h.from_thread_id, h.sent_at
        ));
        let body: String = h.body.chars().take(HANDOFF_BODY_IN_CONTEXT).collect();
        for line in body.lines() {
            s.push_str("  ");
            s.push_str(line);
            s.push('\n');
        }
        if h.body.chars().count() > HANDOFF_BODY_IN_CONTEXT {
            s.push_str(&format!("  … (the rest: `oxplow_read {r}`)\n"));
        }
    }
    s.push_str("</handoffs>");
    Some((s, ids))
}

/// What the session context says around the agent: the other live agents
/// in its stream's worktree but `except` (the asking session), and the
/// open handoffs to its thread.
pub async fn surroundings(
    svc: &Services,
    stream: StreamId,
    thread: ThreadId,
    except: Option<AgentSessionId>,
) -> Surroundings {
    use oxplow_db::semantic_layer::SqlCell;
    let open_handoffs = match svc
        .sql
        .query_sql(
            "SELECT count(*) FROM v_handoff WHERE to_thread_id = ?1 AND state = 'open'",
            vec![SqlCell::Int(thread.value())],
            None,
        )
        .await
    {
        Ok(out) => match out.rows.first().and_then(|r| r.first()) {
            Some(SqlCell::Int(n)) => usize::try_from(*n).unwrap_or(0),
            _ => 0,
        },
        Err(err) => {
            warn!(?err, "counting the thread's open handoffs failed");
            0
        }
    };
    Surroundings {
        others: others_in_worktree(svc, stream, except).await,
        open_handoffs,
    }
}

/// The live agents in stream `stream`'s worktree but `except` (the asking
/// session): its open sessions on open threads whose process still runs,
/// named as the person sees them (`v_agent_session.display_name`).
pub async fn others_in_worktree(
    svc: &Services,
    stream: StreamId,
    except: Option<AgentSessionId>,
) -> Vec<OtherAgent> {
    use oxplow_db::semantic_layer::SqlCell;
    let Ok(out) = svc
        .sql
        .query_sql(
            "SELECT s.id, s.display_name, s.harness, s.pid, t.id, t.title
               FROM v_agent_session s JOIN v_thread t ON t.id = s.thread_id
              WHERE t.stream_id = ?1 AND s.closed_at IS NULL AND t.closed_at IS NULL
                AND s.pid IS NOT NULL
              ORDER BY t.sort_index, s.opened_at, s.id",
            vec![SqlCell::Int(stream.value())],
            None,
        )
        .await
    else {
        return Vec::new();
    };
    let int = |c: &SqlCell| match c {
        SqlCell::Int(i) => Some(*i),
        _ => None,
    };
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => t.clone(),
        _ => String::new(),
    };
    out.rows
        .iter()
        .filter(|r| int(&r[0]).map(AgentSessionId::new) != except)
        .filter(|r| {
            int(&r[3])
                .and_then(|pid| u32::try_from(pid).ok())
                .is_some_and(crate::daemon_supervisor::process_alive)
        })
        .filter_map(|r| {
            Some(OtherAgent {
                name: text(&r[1]),
                harness: text(&r[2]),
                thread_id: ThreadId::new(int(&r[4])?),
                thread_title: text(&r[5]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Record `pid` as `session`'s process.
    async fn set_pid(svc: &Services, session: AgentSessionId, pid: u32) {
        svc.db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::set_pid_tx(
                    tx,
                    session,
                    pid,
                    oxplow_domain::Timestamp::now(),
                )
            })
            .await
            .unwrap();
    }

    /// An agent's context names every other live agent in its worktree —
    /// in its own thread or another of the stream — but not one whose
    /// process is gone, one with no process, or itself.
    #[tokio::test]
    async fn an_agent_is_told_who_else_is_live_in_its_worktree() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let stream = svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let other = crate::test_fixtures::new_thread(svc, stream, "other").await;
        let theirs = crate::test_fixtures::open_session(svc, other.id).await;
        let gone = crate::test_fixtures::open_session(svc, other.id).await;
        let _no_process = crate::test_fixtures::open_session(svc, other.id).await;
        // This test process stands in for a live agent; a pid past any the
        // OS hands out, for one that's gone.
        let me = std::process::id();
        set_pid(svc, fx.session, me).await;
        set_pid(svc, theirs, me).await;
        set_pid(svc, gone, i32::MAX as u32).await;

        let mine = AgentContext::default()
            .refreshed_session_context(svc, &fx.thread, Some("s1"), Some(fx.session))
            .await
            .unwrap();
        let line = mine
            .lines()
            .find(|l| l.starts_with("- **Also in this worktree:**"))
            .unwrap_or_else(|| panic!("{mine}"));
        assert!(
            line.contains(&format!("(`claude`, thread other (`{}`))", other.id)),
            "{line}"
        );
        assert_eq!(
            line.matches("(`claude`").count(),
            1,
            "only the live one: {line}"
        );

        // From the other thread, the first session is the one named.
        let theirs_ctx = AgentContext::default()
            .refreshed_session_context(svc, &other.id, Some("s2"), Some(theirs))
            .await
            .unwrap();
        assert!(
            theirs_ctx.contains(&format!("thread Thread (`{}`)", fx.thread)),
            "{theirs_ctx}"
        );
    }

    /// Building a context takes nothing: until it is committed (the agent
    /// has it), a handoff stays undelivered and every baseline stays, so a
    /// delivery that fails costs nothing and the next build carries it all
    /// again; the commit stamps the handoff by id and records the blocks.
    #[tokio::test]
    async fn a_context_that_was_not_delivered_takes_nothing() {
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let to =
            crate::test_fixtures::new_thread(svc, oxplow_domain::StreamId::new(1), "them").await;
        let theirs = crate::test_fixtures::open_session(svc, to.id).await;
        svc.commands
            .run(
                &Actor::Human,
                crate::commands::handoff::SEND,
                serde_json::json!({
                    "from_thread": fx.thread.to_string(),
                    "to_thread": to.id.to_string(),
                    "body": "the parser half is yours",
                }),
                false,
            )
            .await
            .unwrap();
        let ctx = AgentContext::default();
        let build = || ctx.build_context(svc, &to.id, Some("s1"), Some(theirs), None);
        let undelivered = || async {
            svc.db
                .read(|c| {
                    c.query_row(
                        "SELECT count(*) FROM handoff WHERE delivered_at IS NULL",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };

        // The delivery failed: nothing was committed.
        let failed = build().await.unwrap();
        assert!(failed.text.contains("the parser half is yours"));
        assert_eq!(failed.handoff_ids().len(), 1);
        drop(failed);
        assert_eq!(undelivered().await, 1);

        // So the next build carries the handoff, and the session context,
        // whole.
        let retried = build().await.unwrap();
        assert!(retried.text.contains("the parser half is yours"));
        assert!(
            retried.text.contains("<session-context>"),
            "{}",
            retried.text
        );
        ctx.commit(svc, retried).await;
        assert_eq!(undelivered().await, 0);

        // Delivered: nothing changed, nothing to say.
        assert!(build().await.is_none());
    }

    /// The pump tells watchers that what a context reads changed: a
    /// handoff sent to a thread, but not a command that touches nothing
    /// the context reads.
    #[tokio::test]
    async fn a_handoff_tells_the_context_watchers() {
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let to =
            crate::test_fixtures::new_thread(svc, oxplow_domain::StreamId::new(1), "them").await;
        let mut changes = svc.context_changes.subscribe();
        changes.mark_unchanged();
        svc.commands
            .run(
                &Actor::Human,
                crate::commands::handoff::SEND,
                serde_json::json!({
                    "from_thread": fx.thread.to_string(),
                    "to_thread": to.id.to_string(),
                    "body": "yours",
                }),
                false,
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert!(changes.has_changed().unwrap());
    }

    /// What the person collected in the thread's context basket reaches
    /// the next prompt of any agent in the thread once — pages as
    /// mentions, text quoted with where it came from — and the basket is
    /// then empty; a failed delivery (built, never committed) keeps it.
    #[tokio::test]
    async fn the_context_basket_reaches_the_next_prompt_once() {
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let session = crate::test_fixtures::open_session(svc, fx.thread).await;
        for input in [
            serde_json::json!({ "thread": fx.thread.to_string(), "uri": "oxplow:oxplow/commit?sha=abc", "label": "Fix the parser" }),
            serde_json::json!({ "thread": fx.thread.to_string(), "uri": "oxplow:oxplow/file?path=src/a.rs#lines=3-4", "text": "fn a() {}" }),
        ] {
            svc.commands
                .run(
                    &Actor::Human,
                    crate::commands::context_basket::ADD,
                    input,
                    false,
                )
                .await
                .unwrap();
        }
        let ctx = AgentContext::default();
        // Built but not delivered: still waiting.
        let built = ctx
            .build_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap();
        assert!(built.text.contains("<context-basket>"), "{}", built.text);
        drop(built);
        async fn waiting(svc: &crate::Services, thread: ThreadId) -> usize {
            svc.db
                .read(move |c| oxplow_db::thread_context_store::undelivered_tx(c, thread))
                .await
                .unwrap()
                .len()
        }
        assert_eq!(waiting(svc, fx.thread).await, 2);

        let text = ctx
            .prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap();
        assert!(
            text.contains("- [oxplow uri oxplow:oxplow/commit?sha=abc] (Fix the parser)"),
            "{text}"
        );
        assert!(
            text.contains("- Text from [oxplow uri oxplow:oxplow/file?path=src/a.rs#lines=3-4]:\n```\nfn a() {}\n```"),
            "{text}"
        );
        assert_eq!(waiting(svc, fx.thread).await, 0);
        let again = ctx
            .prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap_or_default();
        assert!(!again.contains("<context-basket>"), "{again}");
    }

    /// A handoff to a thread reaches its agent once, at the next prompt —
    /// stamped delivered to that session — and never with a tool call. The
    /// session context counts the thread's open handoffs for later
    /// sessions, which get the count, not the body again.
    #[tokio::test]
    async fn a_handoff_reaches_the_recipients_next_prompt_once() {
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &*fx.svc;
        let to =
            crate::test_fixtures::new_thread(svc, oxplow_domain::StreamId::new(1), "them").await;
        let theirs = crate::test_fixtures::open_session(svc, to.id).await;
        let send = |body: &'static str| {
            svc.commands.run(
                &Actor::Human,
                crate::commands::handoff::SEND,
                serde_json::json!({
                    "from_thread": fx.thread.to_string(),
                    "to_thread": to.id.to_string(),
                    "body": body,
                }),
                false,
            )
        };
        send("the parser half is yours").await.unwrap();
        let ctx = AgentContext::default();

        assert!(ctx.post_tool_context(svc, &to.id, None).await.is_none());
        let first = ctx
            .prompt_context(svc, &to.id, Some("s1"), Some(theirs), None)
            .await
            .unwrap();
        assert!(first.contains("<handoffs>"), "{first}");
        assert!(first.contains("the parser half is yours"), "{first}");
        assert!(first.contains("from thread Thread"), "{first}");
        assert!(first.contains("oxplow.handoff.resolve"), "{first}");
        assert!(
            first.contains("- **Open handoffs to this thread:** 1"),
            "{first}"
        );
        let delivered: Option<i64> = svc
            .db
            .read(|tx| {
                tx.query_row("SELECT delivered_to_session_id FROM handoff", [], |r| {
                    r.get(0)
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(delivered, Some(theirs.value()));

        let again = ctx
            .prompt_context(svc, &to.id, Some("s1"), Some(theirs), None)
            .await
            .unwrap_or_default();
        assert!(!again.contains("<handoffs>"), "{again}");

        // A later session (a restart) learns of it by the count alone.
        let later = AgentContext::default()
            .prompt_context(svc, &to.id, Some("s2"), Some(theirs), None)
            .await
            .unwrap();
        assert!(!later.contains("the parser half is yours"), "{later}");
        assert!(
            later.contains("- **Open handoffs to this thread:** 1"),
            "{later}"
        );

        // A tool call carries none, even one sent since.
        send("and the lexer").await.unwrap();
        let tool = ctx
            .post_tool_context(svc, &to.id, None)
            .await
            .unwrap_or_default();
        assert!(!tool.contains("and the lexer"), "{tool}");
    }

    /// Alone in the worktree, the context has no such line.
    #[tokio::test]
    async fn the_calling_session_is_not_listed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        set_pid(&fx.svc, fx.session, std::process::id()).await;
        let ctx = AgentContext::default()
            .refreshed_session_context(&fx.svc, &fx.thread, Some("s1"), Some(fx.session))
            .await
            .unwrap();
        assert!(!ctx.contains("Also in this worktree"), "{ctx}");
    }

    /// The session was sent `block` (what a commit records).
    fn sent(state: &AgentContext, session: &str, block: &str) {
        state
            .state
            .lock()
            .last_context_by_session_id
            .insert(session.to_string(), block.to_string());
    }

    #[test]
    fn session_context_emits_initial_and_changed_blocks_only() {
        let state = AgentContext::default();

        assert!(state.would_emit(Some("session-1"), "context-a"));
        sent(&state, "session-1", "context-a");
        assert!(!state.would_emit(Some("session-1"), "context-a"));
        assert!(state.would_emit(Some("session-1"), "context-b"));
    }

    #[test]
    fn session_context_without_session_id_is_never_suppressed() {
        let state = AgentContext::default();
        assert!(state.would_emit(None, "context"));
        assert!(state.would_emit(None, "context"));
    }

    #[test]
    fn clearing_session_context_baseline_allows_fresh_emission() {
        let state = AgentContext::default();
        sent(&state, "session-1", "context");
        assert!(!state.would_emit(Some("session-1"), "context"));
        state.reset_session(Some("session-1"));
        assert!(state.would_emit(Some("session-1"), "context"));
    }
}
