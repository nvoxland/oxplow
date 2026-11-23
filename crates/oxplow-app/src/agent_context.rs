//! The context oxplow hands an agent, shared by every transport (
//! P3.8): the `<session-context>` block (deduped per session — it names
//! the other live agents in the worktree, so it changes as they come and
//! go), the extensions' agent contexts (`context:`, each once, then on
//! change), the context basket and prompt advisories on a prompt; the
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
use oxplow_domain::{AgentSessionId, StreamId, ThreadId};
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

    /// Deliver queued agent nudges. Wait only for an eligible advisory's
    /// consumer to reach the event this hook just logged.
    pub async fn post_tool_context(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        harness: Option<&str>,
        finished_tool_seq: Option<i64>,
    ) -> Option<String> {
        use crate::extension::extensions::{AdvisoryAudience, AdvisoryOn};
        use crate::post_tool_reactors::{IMMEDIATE_ADVISORIES, POST_TOOL_ADVISORIES};

        let mut timer = crate::agent_overhead::Timer::start();
        if !carries(svc, harness, TOOL_CONTEXT) {
            return None;
        }
        let harness = harness.unwrap_or("unknown");
        let wait = svc
            .sql
            .read_generation(async {
                finished_tool_seq.and_then(|seq| {
                    let deps = svc.advisory_deps();
                    let mut consumer = None;
                    crate::advisories::visit_eligible(
                        &deps,
                        AdvisoryOn::PostToolUse,
                        |a| a.audience == AdvisoryAudience::Agent,
                        |_, advisories| {
                            if advisories.iter().any(|a| a.after_collection) {
                                consumer = Some(POST_TOOL_ADVISORIES);
                            } else if consumer.is_none() {
                                consumer = Some(IMMEDIATE_ADVISORIES);
                            }
                        },
                    );
                    consumer.map(|consumer| (consumer, seq))
                })
            })
            .await
            .unwrap_or_default();
        let eligibility_us = timer.lap_us();
        let settled = if let Some((consumer, seq)) = wait {
            svc.event_pump
                .wait_for_hook_event(consumer, seq, POST_TOOL_SETTLE)
                .await
        } else {
            true
        };
        let reactor_wait_us = timer.lap_us();
        let text = undelivered(svc, thread_id).await;
        tracing::debug!(
            target: "oxplow::agent_overhead",
            stage = "post_tool_context",
            harness,
            bytes = text.as_ref().map_or(0, String::len),
            eligibility_us,
            waited_for = wait.map(|(consumer, _)| consumer),
            reactor_wait_us,
            settled,
            nudge_read_us = timer.lap_us(),
            elapsed_us = timer.elapsed_us(),
        );
        text
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
    /// rule, per contribution), the context basket,
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
        let mut timer = crate::agent_overhead::Timer::start();
        if !carries(svc, harness, PROMPT_CONTEXT) {
            return None;
        }
        let harness = harness.unwrap_or("unknown");
        let mut pending = PendingContext {
            text: String::new(),
            baselines: Vec::new(),
            contexts: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session,
            thread: Some(*thread_id),
            overhead_harness: timer.enabled().then(|| harness.to_owned()),
        };
        let ctx_block = self
            .refreshed_session_context(svc, thread_id, session_id, agent_session)
            .await;
        if let (Some(key), Some(block)) = (session_id, &ctx_block) {
            pending.baselines.push((key.to_string(), block.clone()));
        }
        let session_us = timer.lap_us();
        // What the extensions say the agent should know now, each block
        // once, then when it changes.
        let extension_blocks = self
            .changed_extension_contexts(svc, thread_id, session_id)
            .await;
        let extension_count = extension_blocks.len();
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
        let extension_us = timer.lap_us();
        let basket_block = context_basket(svc, *thread_id).await;
        if let Some((_, ids)) = &basket_block {
            pending.basket = ids.clone();
        }
        let basket_us = timer.lap_us();
        // Prompt advisories land as nudges beside any a turn's end or a
        // late tool call left; the prompt carries them all.
        crate::advisories::for_thread(
            &svc.advisory_deps(),
            thread_id,
            crate::extension::extensions::AdvisoryOn::Prompt,
            None,
        )
        .await;
        let advisory_us = timer.lap_us();
        let advisory_block = waiting_nudges(svc, thread_id).await;
        if let Some((_, ids)) = &advisory_block {
            pending.nudges = ids.clone();
        }
        let nudge_read_us = timer.lap_us();
        let sections = [
            ctx_block,
            extension_block,
            basket_block.map(|(text, _)| text),
            advisory_block.map(|(text, _)| text),
        ];
        let [session_bytes, extension_bytes, basket_bytes, nudge_bytes] = sections
            .each_ref()
            .map(|section| section.as_ref().map_or(0, String::len));
        let combined = sections
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
        tracing::debug!(
            target: "oxplow::agent_overhead",
            stage = "prompt_context",
            harness,
            bytes = combined.len(),
            session_bytes,
            session_us,
            extension_bytes,
            extension_count,
            extension_us,
            basket_bytes,
            basket_us,
            advisory_us,
            nudge_bytes,
            nudge_read_us,
            elapsed_us = timer.elapsed_us(),
        );
        if combined.is_empty() {
            return None;
        }
        pending.text = combined;
        Some(pending)
    }

    /// The agent has what [`Self::build_context`] built: remember the
    /// blocks as sent, take the context items it carried, and mark its
    /// nudges delivered.
    pub async fn commit(&self, svc: &Services, built: PendingContext) {
        let timer = crate::agent_overhead::Timer::start();
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
        // The turn this prompt opened (or went into) took the baskets: linked
        // from the turn; the items it carried are deleted and their baskets
        // detached.
        if let (Some(thread), false) = (built.thread, built.basket.is_empty()) {
            let (basket, session) = (built.basket.clone(), built.agent_session.map(|s| s.value()));
            if let Err(err) = svc
                .db
                .transaction(move |tx| {
                    oxplow_db::context_store::take_delivered_tx(tx, thread, session, &basket)
                })
                .await
            {
                warn!(?err, "taking the delivered context baskets failed");
            }
        }
        if !built.nudges.is_empty() {
            if let Err(err) = svc.nudge_store.mark_delivered(built.nudges).await {
                warn!(?err, "marking the delivered nudges failed");
            }
        }
        if let Some(harness) = built.overhead_harness {
            tracing::debug!(
                target: "oxplow::agent_overhead",
                stage = "prompt_commit",
                harness = harness.as_str(),
                bytes = built.text.len(),
                elapsed_us = timer.elapsed_us(),
            );
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
        let around = surroundings(svc, stream.id, agent_session).await;
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
/// baselines, the context items and nudges it carried as delivered.
pub struct PendingContext {
    pub text: String,
    baselines: Vec<(String, String)>,
    /// The extensions' agent contexts it carries: (session, from, block).
    contexts: Vec<(String, String, String)>,
    nudges: Vec<i64>,
    /// The context items it carries (`context_item.id`): the thread's own
    /// basket's, or the named basket's set to go next.
    basket: Vec<i64>,
    agent_session: Option<AgentSessionId>,
    /// The thread it was built for (none for a double in tests).
    thread: Option<ThreadId>,
    overhead_harness: Option<String>,
}

impl PendingContext {
    /// A context that commits nothing: for a double standing in for a
    /// host in tests.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            baselines: Vec::new(),
            contexts: Vec::new(),
            nudges: Vec::new(),
            basket: Vec::new(),
            agent_session: None,
            thread: None,
            overhead_harness: None,
        }
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

/// What the thread's next prompt carries of what the person collected —
/// one basket: its own, or the named basket set to go next for it — as one
/// `<context-basket>` block and the items' ids, still there: they reach the
/// first prompt of any agent in the thread, and [`AgentContext::commit`]
/// deletes them once the agent has the text, which empties that basket and
/// sets the thread back to sending its own.
async fn context_basket(svc: &Services, thread: ThreadId) -> Option<(String, Vec<i64>)> {
    let read = svc
        .db
        .read(move |c| {
            let here: Option<i64> = c
                .query_row(
                    "SELECT stream_id FROM threads WHERE id = ?1",
                    [thread.value()],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)?;
            Ok((
                oxplow_db::context_store::waiting_for_thread_tx(c, thread)?,
                here,
            ))
        })
        .await;
    let (items, here) = match read {
        Ok((items, here)) if !items.is_empty() => (items, here),
        Ok(_) => return None,
        Err(err) => {
            warn!(?err, "reading the thread's context baskets failed");
            return None;
        }
    };
    let ids = items.iter().map(|w| w.item.id).collect();
    Some((basket_block(&items, here), ids))
}

/// The `<context-basket>` block for `items`, numbered so the agent can
/// answer each: each page as a mention the agent reads with `oxplow_read`,
/// each piece of text quoted with where it came from, and the person's note
/// on it, if any, indented under it (never cut: they typed it). A named
/// basket's items follow a line naming it (the person's topic); an item
/// collected in a stream other than `here` says which, so the agent reads
/// its file there.
pub fn basket_block(items: &[oxplow_db::context_store::WaitingItem], here: Option<i64>) -> String {
    use oxplow_db::context_store::ContextKind;
    let mut s = String::from(
        "<context-basket>\nThe person collected these for this prompt while exploring. \
         Read a `[oxplow uri …]` with `oxplow_read`.",
    );
    if items.iter().any(|w| w.item.note.is_some()) {
        s.push_str(
            " Their notes say what they want done about an item: act on each, and say \
             what you did by its number.",
        );
    }
    s.push('\n');
    let mut n = 0;
    let mut basket: Option<&str> = None;
    for waiting in items {
        let item = &waiting.item;
        let label = item
            .label
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        let mut entry = match (item.kind, &item.source_uri, &item.text) {
            (ContextKind::Page, Some(uri), _) => {
                format!("{}{label}\n", prompt_refs::uri_mention(uri))
            }
            (_, source, Some(text)) => {
                let from = source
                    .as_deref()
                    .map(|u| format!(" from {}", prompt_refs::uri_mention(u)))
                    .unwrap_or_default();
                let mut entry = format!("Text{from}{label}:\n{}", fenced(text, ""));
                if let Some(around) = &item.around {
                    entry.push_str(&format!(
                        "   Selected from these lines:\n{}",
                        fenced(around, "   ")
                    ));
                }
                entry
            }
            _ => continue,
        };
        if let (Some(stream), Some(_)) = (&waiting.stream, &item.source_uri) {
            if Some(stream.id) != here {
                let id = oxplow_domain::StreamId::new(stream.id);
                entry.push_str(&format!(
                    "   Collected in stream {id} (\"{}\", branch {}): read it with \
                     `oxplow_read` and `stream: \"{id}\"`.\n",
                    stream.title, stream.branch
                ));
            }
        }
        if waiting.basket.as_deref() != basket {
            basket = waiting.basket.as_deref();
            if let Some(name) = basket {
                s.push_str(&format!("From your \"{name}\" basket:\n"));
            }
        }
        n += 1;
        s.push_str(&format!("{n}. {entry}"));
        if let Some(note) = &item.note {
            // Every line under the item, so a note never reads as the next one.
            for (i, line) in note.lines().enumerate() {
                let lead = if i == 0 { "   Note: " } else { "   " };
                s.push_str(&format!("{lead}{line}\n"));
            }
        }
    }
    s.push_str("</context-basket>");
    s
}

/// `text` in a code fence, cut at `BASKET_TEXT_IN_CONTEXT`, every line
/// led by `indent` (under a list item).
fn fenced(text: &str, indent: &str) -> String {
    let mut text = text.trim_end_matches('\n').to_string();
    if text.len() > BASKET_TEXT_IN_CONTEXT {
        let mut end = BASKET_TEXT_IN_CONTEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n… (cut; read the rest where it came from)");
    }
    let fence = if text.contains("```") { "````" } else { "```" };
    let mut s = format!("{indent}{fence}\n");
    for line in text.lines() {
        s.push_str(&format!("{indent}{line}\n"));
    }
    s.push_str(&format!("{indent}{fence}\n"));
    s
}

/// What the session context says around the agent: the other live agents
/// in its stream's worktree but `except` (the asking session).
pub async fn surroundings(
    svc: &Services,
    stream: StreamId,
    except: Option<AgentSessionId>,
) -> Surroundings {
    Surroundings {
        others: others_in_worktree(svc, stream, except).await,
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

    /// The pump tells watchers that what a context reads changed: an item
    /// put in a thread's basket.
    #[tokio::test]
    async fn a_basket_item_tells_the_context_watchers() {
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
                crate::commands::context_basket::ADD,
                serde_json::json!({ "thread": to.id.to_string(), "uri": "oxplow:oxplow/commit?sha=abc" }),
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
                .read(move |c| oxplow_db::context_store::waiting_for_thread_tx(c, thread))
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
            text.contains("1. [oxplow uri oxplow:oxplow/commit?sha=abc] (Fix the parser)"),
            "{text}"
        );
        assert!(
            text.contains("2. Text from [oxplow uri oxplow:oxplow/file?path=src/a.rs#lines=3-4]:\n```\nfn a() {}\n```"),
            "{text}"
        );
        assert_eq!(waiting(svc, fx.thread).await, 0);
        let again = ctx
            .prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap_or_default();
        assert!(!again.contains("<context-basket>"), "{again}");
    }

    /// What was put back into This Thread waits, held: the next prompt
    /// doesn't carry it (an item added since goes alone) until Send Next on
    /// This Thread; then the prompt after carries it once.
    #[tokio::test]
    async fn a_put_back_waits_until_send_next_on_this_thread() {
        use crate::commands::context_basket::{ADD, PUT_BACK, SEND_NEXT};
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &*fx.svc;
        let session = crate::test_fixtures::open_session(svc, fx.thread).await;
        let thread = fx.thread.to_string();
        let run = |name: &'static str, input: serde_json::Value| async move {
            svc.commands
                .run(&Actor::Human, name, input, false)
                .await
                .unwrap()
        };
        run(
            ADD,
            serde_json::json!({ "thread": thread, "uri": "oxplow:oxplow/commit?sha=abc" }),
        )
        .await;
        let ctx = AgentContext::default();
        let prompt = || async {
            ctx.prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
                .await
                .unwrap_or_default()
        };
        assert!(prompt().await.contains("sha=abc"));

        run(PUT_BACK, serde_json::json!({ "thread": thread })).await;
        assert!(
            !prompt().await.contains("<context-basket>"),
            "put back is held"
        );
        run(
            ADD,
            serde_json::json!({ "thread": thread, "uri": "oxplow:oxplow/commit?sha=new" }),
        )
        .await;
        let with_new = prompt().await;
        assert!(with_new.contains("sha=new"), "{with_new}");
        assert!(!with_new.contains("sha=abc"), "still held: {with_new}");

        run(SEND_NEXT, serde_json::json!({ "thread": thread })).await;
        let released = prompt().await;
        assert!(
            released.contains("1. [oxplow uri oxplow:oxplow/commit?sha=abc]"),
            "{released}"
        );
        assert!(!prompt().await.contains("<context-basket>"), "once");
    }

    /// A named basket set to go next replaces the thread's own for one
    /// prompt, under its name; the prompt empties it and the thread sends
    /// its own again, the basket kept.
    #[tokio::test]
    async fn a_basket_sent_next_replaces_the_threads_own_for_one_prompt() {
        use crate::commands::context_basket::{ADD, SEND_NEXT};
        use oxplow_domain::Actor;
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &*fx.svc;
        let session = crate::test_fixtures::open_session(svc, fx.thread).await;
        let thread = fx.thread.to_string();
        for (name, input) in [
            (
                ADD,
                serde_json::json!({ "basket": "Auth", "uri": "oxplow:oxplow/commit?sha=def" }),
            ),
            (
                ADD,
                serde_json::json!({ "thread": thread, "uri": "oxplow:oxplow/commit?sha=abc" }),
            ),
            (
                SEND_NEXT,
                serde_json::json!({ "basket": "Auth", "thread": thread }),
            ),
        ] {
            svc.commands
                .run(&Actor::Human, name, input, false)
                .await
                .unwrap();
        }
        let ctx = AgentContext::default();
        let text = ctx
            .prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap();
        assert!(
            text.contains("From your \"Auth\" basket:\n1. [oxplow uri oxplow:oxplow/commit?sha=def]\n</context-basket>"),
            "{text}"
        );
        assert!(!text.contains("sha=abc"), "This Thread waits: {text}");
        let basket = svc
            .db
            .read(|c| {
                let b = oxplow_db::context_store::basket_by_name_tx(c, "Auth")?.unwrap();
                let items = oxplow_db::context_store::items_tx(
                    c,
                    oxplow_db::context_store::Basket::Named(b.id),
                )?;
                Ok((b, items.len()))
            })
            .await
            .unwrap();
        assert_eq!((basket.0.next_thread_id, basket.1), (None, 0));
        // The next prompt sends This Thread.
        let after = ctx
            .prompt_context(svc, &fx.thread, Some("s1"), Some(session), None)
            .await
            .unwrap();
        assert!(
            after.contains("1. [oxplow uri oxplow:oxplow/commit?sha=abc]"),
            "{after}"
        );
    }

    fn waiting_item(
        id: i64,
        kind: oxplow_db::context_store::ContextKind,
        uri: &str,
        label: Option<&str>,
        text: Option<&str>,
        note: Option<&str>,
    ) -> oxplow_db::context_store::WaitingItem {
        oxplow_db::context_store::WaitingItem {
            item: oxplow_db::context_store::ContextItem {
                id,
                thread_id: Some(1),
                basket_id: None,
                stream_id: Some(1),
                kind,
                source_uri: Some(uri.into()),
                label: label.map(Into::into),
                text: text.map(Into::into),
                around: None,
                note: note.map(Into::into),
                added_at: "2026-11-18T00:00:00Z".into(),
                held_at: None,
            },
            basket: None,
            stream: Some(oxplow_db::context_store::ItemStream {
                id: 1,
                title: "oxplow".into(),
                branch: "main".into(),
            }),
        }
    }

    /// The basket's items are numbered, so an agent can answer each; a
    /// note follows its item, every line of it indented under it, and the
    /// header says the notes are what the person wants done.
    #[test]
    fn a_basket_items_note_follows_it() {
        use oxplow_db::context_store::ContextKind;
        let block = basket_block(
            &[
                waiting_item(
                    1,
                    ContextKind::Page,
                    "oxplow:oxplow/commit?sha=abc",
                    Some("Fix the parser"),
                    None,
                    None,
                ),
                waiting_item(
                    2,
                    ContextKind::Text,
                    "oxplow:oxplow/file?path=src/a.rs#lines=3-4",
                    None,
                    Some("fn a() {}"),
                    Some("Handle the empty case.\nThen rename it."),
                ),
            ],
            Some(1),
        );
        assert!(
            block.contains("Their notes say what they want done"),
            "{block}"
        );
        assert!(
            block.contains("1. [oxplow uri oxplow:oxplow/commit?sha=abc] (Fix the parser)\n2. "),
            "{block}"
        );
        assert!(
            block.contains(
                "2. Text from [oxplow uri oxplow:oxplow/file?path=src/a.rs#lines=3-4]:\n```\nfn a() {}\n```\n   Note: Handle the empty case.\n   Then rename it.\n</context-basket>"
            ),
            "{block}"
        );
        let plain = basket_block(
            &[waiting_item(
                1,
                ContextKind::Page,
                "oxplow:oxplow/commit?sha=abc",
                None,
                None,
                None,
            )],
            Some(1),
        );
        assert!(
            !plain.contains("notes"),
            "no note, no word about notes: {plain}"
        );
    }

    /// A text from a place with no lines to read back (a terminal) brings
    /// the lines around it, fenced under the item before its note.
    #[test]
    fn a_text_items_lines_around_it_follow_it() {
        use oxplow_db::context_store::ContextKind;
        let mut item = waiting_item(
            1,
            ContextKind::Text,
            "oxplow:oxplow/agent_session?id=ses3",
            None,
            Some("124 of 124"),
            Some("Which suite?"),
        );
        item.item.around = Some("Checks:\n- e2e: the full suite passes, 124 of 124".into());
        let block = basket_block(&[item], Some(1));
        assert!(
            block.contains(
                "1. Text from [oxplow uri oxplow:oxplow/agent_session?id=ses3]:\n```\n124 of 124\n```\n   \
                 Selected from these lines:\n   ```\n   Checks:\n   - e2e: the full suite passes, 124 of 124\n   ```\n   \
                 Note: Which suite?\n</context-basket>"
            ),
            "{block}"
        );
    }

    /// A named basket's items follow a line naming it, numbered on from the
    /// thread's own; one collected in another stream says which and how to
    /// read it there, before its note.
    #[test]
    fn a_named_baskets_items_are_headed_and_say_their_stream() {
        use oxplow_db::context_store::{ContextKind, ItemStream};
        let own = waiting_item(
            1,
            ContextKind::Page,
            "oxplow:oxplow/commit?sha=abc",
            None,
            None,
            None,
        );
        let mut far = waiting_item(
            2,
            ContextKind::Page,
            "oxplow:oxplow/file?path=src/a.rs#lines=3-4",
            None,
            None,
            Some("Why is this here?"),
        );
        far.basket = Some("Auth".into());
        far.stream = Some(ItemStream {
            id: 2,
            title: "Login rework".into(),
            branch: "login".into(),
        });
        let mut near = waiting_item(
            3,
            ContextKind::Page,
            "oxplow:oxplow/commit?sha=def",
            None,
            None,
            None,
        );
        near.basket = Some("Auth".into());
        let block = basket_block(&[own, far, near], Some(1));
        assert!(
            block.contains(
                "1. [oxplow uri oxplow:oxplow/commit?sha=abc]\n\
                 From your \"Auth\" basket:\n\
                 2. [oxplow uri oxplow:oxplow/file?path=src/a.rs#lines=3-4]\n   \
                 Collected in stream str2 (\"Login rework\", branch login): read it with `oxplow_read` and `stream: \"str2\"`.\n   \
                 Note: Why is this here?\n\
                 3. [oxplow uri oxplow:oxplow/commit?sha=def]\n</context-basket>"
            ),
            "{block}"
        );
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
            "permissions: [agent.write]\nconfiguration:\n  version: 1\n  schema: { type: object, additionalProperties: false, properties: { mode: { type: string, enum: [alpha, beta], default: alpha } } }\n",
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
        let fx = crate::test_fixtures::services_with_thread().await;
        write_context(
            &fx.svc,
            "state",
            "permissions: [agent.write]\nconfiguration:\n  version: 1\n  schema: { type: object, additionalProperties: false, properties: { mode: { type: string, enum: [alpha, beta], default: alpha } } }\n",
            "  - id: shown\n    doc: Said in alpha.\n    when: { config: /mode, in: [alpha] }\n    query: SELECT 'alpha says' AS text\n  - id: snaps\n    doc: Said with snapshots.\n    needs: [snapshots.contents]\n    query: SELECT 'snapped' AS text\n",
        );
        crate::test_fixtures::activate(&fx.svc).await;
        let ctx = AgentContext::default();
        assert_eq!(prompt(&ctx, &fx).await.len(), 2);
        let extensions = fx
            .svc
            .extension_catalog
            .live_snapshot(&fx.svc.layout.project_dir);
        crate::settings::store(
            &fx.svc.layout.project_dir,
            &fx.svc.global_config_dir,
            fx.svc.secrets.clone(),
            extensions.extensions(),
        )
        .unwrap()
        .set(
            "state.mode",
            serde_json::json!("beta"),
            oxplow_config::settings::Level::Project,
        )
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
