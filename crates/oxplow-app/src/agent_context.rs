//! The context oxplow hands an agent, shared by every transport (
//! P3.8): the `<session-context>` block (deduped per session — it names
//! the other live agents in the worktree, so it changes as they come and
//! go), the extensions' agent contexts (`context:`, each once, then on
//! change), the open handoffs to the thread no agent was given yet, prompt
//! advisories on a prompt; the
//! thread's undelivered nudges after a tool call. What an agent *did* is
//! recorded elsewhere — the hook ingest logs `agent.*` events and pump
//! reactors record from them (`tool_call_reactors`, `post_tool_reactors`,
//! the token reactor).
//!
//! State kept here is runtime-only (losing it on a restart costs at most
//! one repeated context block): the last context and the last block of
//! each extension's agent context per agent session. The
//! session's resume id is the hook ingest's (`hook_ingest`). See
//! `.context/agent-model.md`.

use std::collections::HashMap;

use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::uri::prompt_refs;
use oxplow_domain::{AgentSessionId, HandoffId, StreamId, ThreadId};
use parking_lot::Mutex;
use tracing::warn;

use crate::agent_prompt::{build_session_context_block, OtherAgent, Surroundings};
use crate::Services;

/// What each agent session was last sent, per block.
#[derive(Default)]
struct ContextState {
    /// Last context block returned per session. The launch prompt already carries this data, so a
    /// byte-identical repeat adds noise without new information.
    last_context_by_session_id: HashMap<String, String>,
    /// Per session, the last block each extension's agent context sent
    /// (by `<namespace>/<id>`): sent again only when it changes.
    last_extension_context: HashMap<String, HashMap<String, String>>,
}

/// How long a tool call's hook waits for the reactors that write nudges.
const POST_TOOL_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

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
        state.last_extension_context.remove(session_id);
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
    /// what the session was last sent), each extension's agent context (the same
    /// rule, per contribution), the thread's undelivered handoffs,
    /// nudges (prompt advisories, and the hints its last turn's end left).
    /// Nothing changes until [`Self::commit`] — the delivery can
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
            contexts: Vec::new(),
            handoffs: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session,
            thread: Some(*thread_id),
        };
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id, agent_session)
            .await;
        if let (Some(key), Some(block)) = (session_id, &ctx_block) {
            pending.baselines.push((key.to_string(), block.clone()));
        }
        // What the extensions say the agent should know now, each block
        // once, then when it changes.
        let extension_blocks = self
            .changed_extension_contexts(svc, thread_id, session_id)
            .await;
        if let Some(key) = session_id {
            pending.contexts = extension_blocks
                .iter()
                .map(|(from, block)| (key.to_string(), from.clone(), block.clone()))
                .collect();
        }
        let extension_block = (!extension_blocks.is_empty()).then(|| {
            extension_blocks
                .into_iter()
                .map(|(_, block)| block)
                .collect::<Vec<_>>()
                .join("\n\n")
        });
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
            crate::extension::extensions::AdvisoryOn::Prompt,
            None,
        )
        .await;
        let advisory_block = waiting_nudges(svc, thread_id).await;
        if let Some((_, ids)) = &advisory_block {
            pending.nudges = ids.clone();
        }
        let combined: String = [
            ctx_block,
            extension_block,
            handoffs_block.map(|(text, _)| text),
            basket_block.map(|(text, _)| text),
            advisory_block.map(|(text, _)| text),
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
            for (session, from, block) in built.contexts {
                state
                    .last_extension_context
                    .entry(session)
                    .or_default()
                    .insert(from, block);
            }
        }
        if !built.handoffs.is_empty() {
            if let Err(err) = svc
                .handoff_store
                .mark_delivered(built.handoffs.clone(), built.agent_session)
                .await
            {
                warn!(?err, "stamping the delivered handoffs failed");
            }
        }
        // The turn this prompt opened (or went into) took the handoffs and
        // the basket: on the record, and linked from the turn.
        if let (Some(thread), false) = (
            built.thread,
            built.handoffs.is_empty() && built.basket.is_empty(),
        ) {
            let (handoffs, basket, session) = (
                built.handoffs.clone(),
                built.basket.clone(),
                built.agent_session.map(|s| s.value()),
            );
            if let Err(err) = svc
                .db
                .transaction(move |tx| {
                    oxplow_db::thread_context_store::link_delivered_tx(
                        tx, thread, session, &handoffs, &basket,
                    )
                })
                .await
            {
                warn!(?err, "linking the delivered context to its turn failed");
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

    /// The extensions' agent contexts the session wasn't sent as they read
    /// now, each as its block, by who says it: one cut to
    /// [`CONTEXT_BUDGET`], then all of them to [`CONTEXTS_BUDGET`]. What
    /// doesn't fit at all waits for a later prompt, when the ones sent
    /// now don't repeat.
    async fn changed_extension_contexts(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        session_id: Option<&str>,
    ) -> Vec<(String, String)> {
        // Nothing is known when the generation can't be read: nothing new is
        // said and nothing taken back.
        let Ok(current) = extension_contexts(svc, thread_id).await else {
            return Vec::new();
        };
        // One the session was sent that is silent now (or gone, with its
        // extension) tells it so, once: what it said no longer holds. One
        // that failed takes nothing back. Recorded as sent like any block,
        // so the same text coming back differs from it and is news again.
        let still = |from: &str| {
            current
                .iter()
                .any(|c| c.from == from && !matches!(c.reading, Reading::Silent))
        };
        let mut cleared: Vec<(String, String)> = session_id
            .and_then(|session| {
                let state = self.state.lock();
                state.last_extension_context.get(session).map(|sent| {
                    sent.iter()
                        .filter(|(from, _)| !still(from))
                        .map(|(from, previous)| (from.clone(), previous.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(from, previous)| {
                let block = context_block(&from, CONTEXT_CLEARED, false);
                (previous != block).then_some((from, block))
            })
            .collect();
        cleared.sort();
        let changed: Vec<(String, String, bool)> = current
            .into_iter()
            .filter_map(|c| match c.reading {
                Reading::Says(text) => {
                    let (text, cut) = cut_to(&text, CONTEXT_BUDGET);
                    Some((c.from, text, cut))
                }
                Reading::Silent | Reading::Failed(_) => None,
            })
            .filter(|(from, text, cut)| {
                self.would_emit_extension_context(
                    session_id,
                    from,
                    &context_block(from, text, *cut),
                )
            })
            .collect();
        let mut left = CONTEXTS_BUDGET;
        let mut out = cleared;
        for (from, text, cut) in changed {
            if left == 0 {
                break;
            }
            let (text, over) = cut_to(&text, left);
            left -= text.chars().count();
            let block = context_block(&from, &text, cut || over);
            out.push((from, block));
        }
        out
    }

    /// Whether `block` is new from `from` for `session_id` (as
    /// [`Self::would_emit`] for the session context).
    fn would_emit_extension_context(
        &self,
        session_id: Option<&str>,
        from: &str,
        block: &str,
    ) -> bool {
        let Some(session_id) = session_id else {
            return true;
        };
        self.state
            .lock()
            .last_extension_context
            .get(session_id)
            .and_then(|sent| sent.get(from))
            .is_none_or(|previous| previous != block)
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
    /// The extensions' agent contexts it carries: (session, from, block).
    contexts: Vec<(String, String, String)>,
    handoffs: Vec<HandoffId>,
    nudges: Vec<i64>,
    /// The context basket's items it carries (`thread_context_item.id`).
    basket: Vec<i64>,
    agent_session: Option<AgentSessionId>,
    /// The thread it was built for (none for a double in tests).
    thread: Option<ThreadId>,
}

impl PendingContext {
    /// A context that commits nothing: for a double standing in for a
    /// host in tests.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            baselines: Vec::new(),
            contexts: Vec::new(),
            handoffs: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session: None,
            thread: None,
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

/// Characters one extension's agent context may say on a prompt; the
/// rest is cut, with [`CONTEXT_CUT`].
pub const CONTEXT_BUDGET: usize = 1_500;
/// Characters all extensions' agent contexts may say on one prompt.
pub const CONTEXTS_BUDGET: usize = 3_000;
/// The line that ends a cut agent context.
pub const CONTEXT_CUT: &str = "… (cut: over its budget)";
/// What an agent context the session was sent says once it has nothing to
/// say: what it said before no longer holds.
pub const CONTEXT_CLEARED: &str = "Nothing now: what this said before no longer holds.";

/// What one extension's agent context reads for a thread now: who says
/// it (`<namespace>/<id>`) and its [`Reading`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionContext {
    pub from: String,
    pub reading: Reading,
}

/// An agent context's reading. Only `Silent` takes back what a session
/// was sent: a reading that failed says nothing about the facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Its text, trimmed and uncut.
    Says(String),
    /// Nothing to say: no row, an empty text, its `needs` or `when:` not met.
    Silent,
    /// Its query or its settings couldn't be read (why).
    Failed(String),
}

/// Every enabled extension's agent context (`context:`) for `thread`, read
/// now: what its first row's `text` says (trimmed), silent (no row, an
/// empty text, its `needs` or `when:` not met), or failed (logged). By
/// namespace, then as each extension declares them. `Err` when the
/// generation can't be read: then nothing is known. Not a nudge: nothing
/// is recorded, muted or counted.
pub async fn extension_contexts(
    svc: &Services,
    thread: &ThreadId,
) -> Result<Vec<ExtensionContext>, String> {
    svc.sql
        .read_generation(extension_contexts_in_generation(svc, thread))
        .await
        .map_err(|error| {
            warn!(%error, "agent context generation read failed");
            error.to_string()
        })
}

async fn extension_contexts_in_generation(
    svc: &Services,
    thread: &ThreadId,
) -> Vec<ExtensionContext> {
    use crate::extension::extensions::agent_contexts::TEXT_COLUMN;
    use oxplow_db::semantic_layer::SqlCell;
    let active = svc
        .capabilities
        .snapshot(&crate::config_service::read_config(&svc.config));
    let mut extensions: Vec<crate::extension::extensions::Extension> = svc
        .extension_catalog
        .get(svc.worktrees.project_dir())
        .iter()
        .filter(|e| e.enabled && !e.contexts.is_empty())
        .cloned()
        .collect();
    if extensions.is_empty() {
        return Vec::new();
    }
    extensions.sort_by(|a, b| a.namespace.cmp(&b.namespace));
    let stream = match svc.thread_store.get(thread).await {
        Ok(Some(t)) => SqlCell::Int(t.stream_id.value()),
        _ => SqlCell::Null(()),
    };
    let request = crate::advisories::request_of(&svc.sql, thread, None)
        .await
        .map_or(SqlCell::Null(()), SqlCell::Int);
    // The settings that offer contexts (`when:`), read once.
    let settings = crate::capabilities::offering_settings(svc, &extensions).await;
    let params = vec![
        ("thread_id".to_string(), SqlCell::Int(thread.value())),
        ("stream_id".to_string(), stream),
        ("request_id".to_string(), request),
    ];
    let mut out = Vec::new();
    for ext in &extensions {
        for c in &ext.contexts {
            let from = format!("{}/{}", ext.namespace, c.id);
            let mut read = |reading| {
                out.push(ExtensionContext {
                    from: from.clone(),
                    reading,
                })
            };
            if !active.unmet(&c.needs).is_empty() {
                read(Reading::Silent);
                continue;
            }
            if let Some(when) = &c.when {
                // A `when:` needs the extension's configuration, so settings
                // missing here are settings that couldn't be read.
                match settings.get(&ext.namespace) {
                    None => {
                        read(Reading::Failed("its settings can't be read".into()));
                        continue;
                    }
                    Some(values) if !when.holds(values) => {
                        read(Reading::Silent);
                        continue;
                    }
                    Some(_) => {}
                }
            }
            let query = match svc.sql.with_extension_config(ext, &c.query).await {
                Ok(query) => query,
                Err(error) => {
                    warn!(context = %from, %error, "agent context configuration failed");
                    read(Reading::Failed(error.to_string()));
                    continue;
                }
            };
            let result = match svc
                .sql
                .run(
                    oxplow_db::SqlQuery::new(&query)
                        .named(params.clone())
                        .limit(Some(1)),
                )
                .await
            {
                Ok(r) => r,
                Err(error) => {
                    warn!(context = %from, %error, "agent context query failed");
                    read(Reading::Failed(error.to_string()));
                    continue;
                }
            };
            let Some(i) = result.columns.iter().position(|c| c == TEXT_COLUMN) else {
                warn!(context = %from, "agent context query returned no `text` column");
                read(Reading::Failed(
                    "its query returned no `text` column".into(),
                ));
                continue;
            };
            let text = match result.rows.first().map(|r| &r[i]) {
                Some(SqlCell::Text(t)) => t.trim().to_string(),
                _ => String::new(),
            };
            read(if text.is_empty() {
                Reading::Silent
            } else {
                Reading::Says(text)
            });
        }
    }
    out
}

/// `text` within `limit` characters, and whether it was cut.
fn cut_to(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    (text.chars().take(limit).collect(), true)
}

/// The block an agent context is sent as.
fn context_block(from: &str, text: &str, cut: bool) -> String {
    let marker = if cut {
        format!("\n{CONTEXT_CUT}")
    } else {
        String::new()
    };
    format!("<context from=\"{from}\">\n{text}{marker}\n</context>")
}

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
                s.push_str(&format!("- {}{label}\n", prompt_refs::uri_mention(uri)));
            }
            (_, source, Some(text)) => {
                let from = source
                    .as_deref()
                    .map(|u| format!(" from {}", prompt_refs::uri_mention(u)))
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
    // Its task's title through the task resource's own dataset.
    let (task_view, task_title) =
        crate::ref_resolver::resolve_view(svc, oxplow_domain::uri::resources::TASK)
            .unwrap_or_else(|| ("(SELECT NULL AS uri, NULL AS title)".into(), "title".into()));
    let labels: HashMap<String, (String, Option<String>, Option<String>)> = match svc
        .sql
        .query_sql(
            &format!(
                "SELECT h.uri, t.title, s.display_name, w.{task_title}
                   FROM v_handoff h
                   JOIN v_thread t ON t.id = h.from_thread_id
                   LEFT JOIN v_agent_session s ON s.id = h.from_session_id
                   LEFT JOIN {task_view} w ON w.uri = h.task
                  WHERE h.to_thread_id = ?1"
            ),
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
         moves its task onto this thread, and its sender reads the note.\n",
    );
    for h in taken {
        let r = oxplow_domain::uri::build::handoff_uri(h.id);
        let (from_title, from_name, item_title) = labels.get(&r).cloned().unwrap_or_default();
        let by = from_name.unwrap_or_else(|| "the person".into());
        let item = item_title
            .map(|t| format!("; task \"{t}\""))
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
        let fx = crate::test_fixtures::services_with_thread().await;
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
        let fx = crate::test_fixtures::services_with_thread().await;
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
        let fx = crate::test_fixtures::services_with_thread().await;
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

    /// While an extension declares an agent context (oxtask's task state),
    /// any command may change what it reads: a task assigned to the thread
    /// tells the watchers, so a harness with no prompt hook (Codex) has it
    /// before the next turn.
    #[tokio::test]
    async fn a_command_tells_the_context_watchers_while_a_context_is_declared() {
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &*fx.svc;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: d.\n    query: SELECT 'x' AS text\n",
        )
        .await;
        let mut changes = svc.context_changes.subscribe();
        changes.mark_unchanged();
        svc.commands
            .run(
                &Actor::Human,
                "oxplow.thread.rename",
                serde_json::json!({ "thread": fx.thread.to_string(), "title": "Renamed" }),
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
        let fx = crate::test_fixtures::services_with_thread().await;
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
        let fx = crate::test_fixtures::services_with_thread().await;
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

    /// A project extension `name` declaring `context` (YAML list items),
    /// with `agent.write`, activated.
    async fn with_context(svc: &std::sync::Arc<Services>, name: &str, context: &str) {
        write_context(svc, name, "permissions: [agent.write]\n", context);
        crate::test_fixtures::activate(svc).await;
    }

    fn write_context(svc: &Services, name: &str, permissions: &str, context: &str) {
        let dir = svc.layout.project_dir.join(".oxplow/extensions").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!(
                "manifest: 2\nname: {name}\nintent:\n  purpose: test\n{permissions}context:\n{context}"
            ),
        )
        .unwrap();
    }

    /// The `<context from=..>` blocks of a prompt's context, in order: the
    /// test's own extensions', not the shipped ones' (`oxplow/`, oxtask's
    /// task state).
    fn contexts(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("<context from=") {
            let end = rest[start..].find("</context>").unwrap() + start + "</context>".len();
            if !rest[start..].starts_with("<context from=\"oxplow/") {
                out.push(rest[start..end].to_string());
            }
            rest = &rest[end..];
        }
        out
    }

    async fn prompt(ctx: &AgentContext, fx: &crate::test_fixtures::ThreadFixture) -> Vec<String> {
        contexts(
            &ctx.prompt_context(&fx.svc, &fx.thread, Some("s1"), Some(fx.session), None)
                .await
                .unwrap_or_default(),
        )
    }

    async fn retitle(svc: &Services, thread: ThreadId, title: &'static str) {
        svc.db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE threads SET title = ?1 WHERE id = ?2",
                    rusqlite::params![title, thread.value()],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// An extension's context reaches the session's first prompt, tagged
    /// with where it came from; an unchanged text isn't sent again, a
    /// changed one is.
    #[tokio::test]
    async fn an_extension_context_is_sent_once_then_on_change() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: The thread's title.\n    query: SELECT 'working on ' || title AS text FROM v_thread WHERE id = :thread_id\n",
        )
        .await;
        let ctx = AgentContext::default();
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec!["<context from=\"state/now\">\nworking on Thread\n</context>".to_string()]
        );
        assert_eq!(prompt(&ctx, &fx).await, Vec::<String>::new());
        retitle(&fx.svc, fx.thread, "Parser").await;
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec!["<context from=\"state/now\">\nworking on Parser\n</context>".to_string()]
        );
        assert_eq!(prompt(&ctx, &fx).await, Vec::<String>::new());
    }

    /// A compaction (or a fresh start) forgets what the session was sent:
    /// the next prompt carries the context again.
    #[tokio::test]
    async fn a_compaction_sends_the_extension_context_again() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: A fact.\n    query: SELECT 'a fact' AS text\n",
        )
        .await;
        let ctx = AgentContext::default();
        assert_eq!(prompt(&ctx, &fx).await.len(), 1);
        assert!(prompt(&ctx, &fx).await.is_empty());
        ctx.reset_session(Some("s1"));
        assert_eq!(prompt(&ctx, &fx).await.len(), 1);
    }

    /// A context that stops saying anything tells the session once that
    /// what it said no longer holds (an agent told of a task in progress
    /// learns it closed); a session never told of it hears nothing. When it
    /// says the same thing again later, it's sent again.
    #[tokio::test]
    async fn an_extension_context_that_went_away_says_so_once_and_is_sent_again_when_it_returns() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: The thread's title, unless it's Gone.\n    query: SELECT 'on ' || title AS text FROM v_thread WHERE id = :thread_id AND title <> 'Gone'\n",
        )
        .await;
        let ctx = AgentContext::default();
        assert_eq!(prompt(&ctx, &fx).await.len(), 1);
        retitle(&fx.svc, fx.thread, "Gone").await;
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec![format!(
                "<context from=\"state/now\">\n{CONTEXT_CLEARED}\n</context>"
            )]
        );
        assert!(prompt(&ctx, &fx).await.is_empty(), "said once");
        // A fresh session was never told of it: nothing to take back.
        ctx.reset_session(Some("s1"));
        assert!(prompt(&ctx, &fx).await.is_empty(), "nothing to clear");
        retitle(&fx.svc, fx.thread, "Thread").await;
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec!["<context from=\"state/now\">\non Thread\n</context>".to_string()]
        );
    }

    /// A context whose query fails says nothing new and takes nothing
    /// back: the session keeps what it was told, and once the query reads
    /// again unchanged, nothing is repeated.
    #[tokio::test]
    async fn a_failed_extension_context_takes_nothing_back() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: The thread's title; Broken fails.\n    query: SELECT CASE WHEN title = 'Broken' THEN json_extract('not json', '$') ELSE 'on ' || title END AS text FROM v_thread WHERE id = :thread_id\n",
        )
        .await;
        let ctx = AgentContext::default();
        assert_eq!(prompt(&ctx, &fx).await.len(), 1);
        retitle(&fx.svc, fx.thread, "Broken").await;
        assert!(
            prompt(&ctx, &fx).await.is_empty(),
            "no retraction on a failure"
        );
        retitle(&fx.svc, fx.thread, "Thread").await;
        assert!(
            prompt(&ctx, &fx).await.is_empty(),
            "the session still holds it"
        );
    }

    /// A context the extension's own setting offers (`when:`) is sent only
    /// while the setting holds; the defaults count when nothing is stored.
    #[tokio::test]
    async fn an_extension_context_follows_its_setting() {
        let fx = crate::test_fixtures::services_with_thread().await;
        write_context(
            &fx.svc,
            "state",
            "permissions: [agent.write]\nconfiguration:\n  version: 1\n  schema: { type: object, additionalProperties: false, properties: { mode: { type: string, enum: [alpha, beta] } } }\n  defaults: { mode: alpha }\n",
            "  - id: shown\n    doc: Said in alpha.\n    when: { config: /mode, in: [alpha] }\n    query: SELECT 'alpha says' AS text\n  - id: hidden\n    doc: Said in beta.\n    when: { config: /mode, in: [beta] }\n    query: SELECT 'beta says' AS text\n",
        );
        crate::test_fixtures::activate(&fx.svc).await;
        assert_eq!(
            prompt(&AgentContext::default(), &fx).await,
            vec!["<context from=\"state/shown\">\nalpha says\n</context>".to_string()]
        );
    }

    /// A stored setting that turns a context off takes it back once, and
    /// one whose `needs` stop being met (no snapshot contents) does too.
    #[tokio::test]
    async fn a_context_turned_off_by_its_setting_or_its_needs_is_taken_back() {
        use oxplow_db::extension_config_store::{ConfigNamespace, SqliteExtensionConfigStore};
        let fx = crate::test_fixtures::services_with_thread().await;
        write_context(
            &fx.svc,
            "state",
            "permissions: [agent.write]\nconfiguration:\n  version: 1\n  schema: { type: object, additionalProperties: false, properties: { mode: { type: string, enum: [alpha, beta] } } }\n  defaults: { mode: alpha }\n",
            "  - id: shown\n    doc: Said in alpha.\n    when: { config: /mode, in: [alpha] }\n    query: SELECT 'alpha says' AS text\n  - id: snaps\n    doc: Said with snapshots.\n    needs: [snapshots.contents]\n    query: SELECT 'snapped' AS text\n",
        );
        crate::test_fixtures::activate(&fx.svc).await;
        let ctx = AgentContext::default();
        assert_eq!(prompt(&ctx, &fx).await.len(), 2);
        SqliteExtensionConfigStore::new(fx.svc.db.clone())
            .compare_exchange(
                ConfigNamespace::new(
                    "state".into(),
                    oxplow_domain::extension_state::Scope::Project,
                )
                .unwrap(),
                None,
                1,
                serde_json::json!({ "mode": "beta" }),
            )
            .await
            .unwrap();
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec![format!(
                "<context from=\"state/shown\">\n{CONTEXT_CLEARED}\n</context>"
            )]
        );
        fx.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.capability.choose",
                serde_json::json!({ "capability": "snapshots", "provider": "hashes" }),
                true,
            )
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        assert!(
            !fx.svc
                .capabilities
                .snapshot(&crate::config_service::read_config(&fx.svc.config))
                .unmet(&["snapshots.contents".to_string()])
                .is_empty(),
            "no snapshot contents now"
        );
        assert_eq!(
            prompt(&ctx, &fx).await,
            vec![format!(
                "<context from=\"state/snaps\">\n{CONTEXT_CLEARED}\n</context>"
            )]
        );
    }

    /// A query that returns no row, or an empty text, has nothing to say.
    #[tokio::test]
    async fn an_extension_context_without_a_row_says_nothing() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: none\n    doc: Nothing.\n    query: SELECT 'x' AS text WHERE 0\n  - id: blank\n    doc: Blank.\n    query: SELECT '  ' AS text\n",
        )
        .await;
        assert!(prompt(&AgentContext::default(), &fx).await.is_empty());
    }

    /// Several contributions all arrive, in a stable order: by namespace,
    /// then as each extension declares them.
    #[tokio::test]
    async fn two_extension_contexts_arrive_in_a_stable_order() {
        let fx = crate::test_fixtures::services_with_thread().await;
        write_context(
            &fx.svc,
            "zeta",
            "permissions: [agent.write]\n",
            "  - id: b\n    doc: B.\n    query: SELECT 'zb' AS text\n  - id: a\n    doc: A.\n    query: SELECT 'za' AS text\n",
        );
        with_context(
            &fx.svc,
            "alpha",
            "  - id: one\n    doc: One.\n    query: SELECT 'a1' AS text\n",
        )
        .await;
        let got: Vec<String> = prompt(&AgentContext::default(), &fx)
            .await
            .iter()
            .map(|b| b.lines().next().unwrap().to_string())
            .collect();
        assert_eq!(
            got,
            vec![
                "<context from=\"alpha/one\">",
                "<context from=\"zeta/b\">",
                "<context from=\"zeta/a\">",
            ]
        );
    }

    /// One contribution says at most `CONTEXT_BUDGET` characters, and all
    /// of them together `CONTEXTS_BUDGET`: the rest is cut, visibly.
    #[tokio::test]
    async fn extension_contexts_are_cut_to_their_budget() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let long = |n: usize| format!("SELECT replace(printf('%{n}s', ''), ' ', 'x') AS text");
        with_context(
            &fx.svc,
            "big",
            &format!(
                "  - id: a\n    doc: A.\n    query: \"{}\"\n  - id: b\n    doc: B.\n    query: \"{}\"\n  - id: c\n    doc: C.\n    query: \"{}\"\n",
                long(2_000),
                long(1_400),
                long(1_400)
            ),
        )
        .await;
        let blocks = prompt(&AgentContext::default(), &fx).await;
        let body = |b: &str| b.lines().nth(1).unwrap().to_string();
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(body(&blocks[0]).len(), CONTEXT_BUDGET);
        assert!(blocks[0].contains(CONTEXT_CUT), "{}", blocks[0]);
        assert_eq!(body(&blocks[1]).len(), 1_400);
        assert!(!blocks[1].contains(CONTEXT_CUT));
        assert_eq!(
            body(&blocks[2]).len(),
            CONTEXTS_BUDGET - CONTEXT_BUDGET - 1_400
        );
        assert!(blocks[2].contains(CONTEXT_CUT), "{}", blocks[2]);
    }

    /// What reaches the agent's context needs `agent.write`: without it
    /// the extension doesn't load, and the agent hears nothing.
    #[tokio::test]
    async fn an_extension_context_needs_agent_write() {
        let fx = crate::test_fixtures::services_with_thread().await;
        write_context(
            &fx.svc,
            "sneaky",
            "",
            "  - id: now\n    doc: A fact.\n    query: SELECT 'a fact' AS text\n",
        );
        let errors = crate::test_fixtures::load_errors(&fx.svc, "sneaky").await;
        assert!(
            errors.contains("context `now` needs `agent.write` in `permissions:`"),
            "{errors}"
        );
        assert!(prompt(&AgentContext::default(), &fx).await.is_empty());
    }

    /// A harness that can't carry context on a prompt gets none, and
    /// nothing is recorded as sent: the session's first prompt through a
    /// channel still carries it.
    #[tokio::test]
    async fn a_harness_without_the_prompt_channel_gets_no_extension_context() {
        let fx = crate::test_fixtures::services_with_thread().await;
        with_context(
            &fx.svc,
            "state",
            "  - id: now\n    doc: A fact.\n    query: SELECT 'a fact' AS text\n",
        )
        .await;
        let ctx = AgentContext::default();
        assert!(ctx
            .prompt_context(
                &fx.svc,
                &fx.thread,
                Some("s1"),
                Some(fx.session),
                Some("no-prompt-channel")
            )
            .await
            .is_none());
        assert_eq!(prompt(&ctx, &fx).await.len(), 1);
    }

    /// Alone in the worktree, the context has no such line.
    #[tokio::test]
    async fn the_calling_session_is_not_listed() {
        let fx = crate::test_fixtures::services_with_thread().await;
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
