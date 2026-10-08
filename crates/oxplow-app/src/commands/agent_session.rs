//! Agent-session commands (`.context/commands.md`): the agent slots a
//! person opens on a thread — open, rename, close — as `Tx` commands.
//!
//! **The command is the slot's lifecycle; the RPC is the process's.**
//! Opening a session only inserts its row: the UI sees it and starts its
//! process (a PTY or an ACP agent), and nothing types into it — the
//! no-automation guards keep that true for a session an agent opened too.
//! Closing one closes its slot and, once that commits, stops its process
//! (`SessionProcesses`). Stopping a process alone (`terminate_terminal_session`,
//! `acp_close_session`) leaves the slot open.
//!
//! A session is named by ref (`agent_session:ses3`), a thread by
//! `thread:thr1`. An agent acts only on sessions in its own stream, and its
//! close — destructive — waits for a person as a proposal.

use crate::commands::ops::Op;
use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;
use oxplow_db::agent_session_store::{get_tx, insert_tx, set_title_tx};
use oxplow_domain::agent::registry::{AcpAdapterRegistry, HarnessRegistry};
use oxplow_domain::agent_session::{
    AgentSession, NewAgentSession, SessionCloseReason, SessionKind,
};
use oxplow_domain::refs::build::{agent_session_ref, stream_ref, thread_ref};
use oxplow_domain::{
    AgentSessionId, CommandCall, CommandError, Confirm, ThreadId, ThreadStatus, Timestamp,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, TxCtx};

pub const OPEN: &str = "oxplow.agent_session.open";
pub const RENAME: &str = "oxplow.agent_session.rename";
pub const CLOSE: &str = "oxplow.agent_session.close";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    /// The thread (`thread:thr1`).
    pub thread: String,
    /// `terminal` or `chat` (default: `chat` for a harness with a
    /// structured transcript, such as `acp`, else `terminal`).
    #[serde(default)]
    pub kind: Option<SessionKind>,
    /// What runs in it: a registered harness's key (default: the project's
    /// first enabled agent).
    #[serde(default)]
    pub harness: Option<String>,
    /// For a chat session, which ACP agent.
    #[serde(default)]
    pub acp_agent: Option<String>,
    /// What to call it.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The session (`agent_session:ses3`).
    pub session: String,
    pub title: String,
}

fn call(name: &str, input: serde_json::Value) -> Option<CommandCall> {
    Some(CommandCall {
        name: name.into(),
        input,
    })
}

fn result(session: &AgentSession) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(session).expect("a session serializes"),
        ..HandlerOutput::default()
    }
}

/// The session an `agent_session:<id>` ref names.
fn load(ctx: &TxCtx<'_>, value: &str) -> Result<AgentSession, CommandError> {
    let id: AgentSessionId = ref_id(value, "agent_session", "/session")?;
    get_tx(ctx.conn, id)?.ok_or_else(|| invalid("/session", format!("no agent session `{value}`")))
}

/// An agent acts only on sessions of threads in its own stream.
fn in_own_stream(ctx: &TxCtx<'_>, thread: ThreadId) -> Result<(), CommandError> {
    let Some((_, own)) = super::thread::agent_scope(ctx)? else {
        return Ok(());
    };
    let stream = oxplow_db::thread_store::get_tx(ctx.conn, thread)
        .map_err(sql)?
        .map(|t| t.stream_id);
    if stream != Some(own) {
        return Err(CommandError::Denied {
            reason: format!(
                "an agent changes agent sessions only in its own stream (`{}`)",
                stream_ref(own)
            ),
        });
    }
    Ok(())
}

/// The harness, kind and ACP agent a session runs: what was named, checked
/// against the registered harnesses, the project's enabled ones and its
/// ACP agents, else the project's default. A harness with a structured
/// transcript runs an ACP agent in a chat; the rest run in a terminal.
fn choose_agent(
    config: &OxplowConfig,
    harnesses: &HarnessRegistry,
    adapters: &AcpAdapterRegistry,
    input: &OpenInput,
) -> Result<(String, SessionKind, Option<String>), CommandError> {
    let harness = match &input.harness {
        Some(key) => harnesses.get(key),
        None => harnesses.default(),
    }
    .map_err(|e| invalid("/harness", e.to_string()))?;
    let key = harness.id().to_string();
    if !config.agents.is_empty() && !config.agents.contains(&key) {
        return Err(invalid(
            "/harness",
            format!("agent `{key}` isn't enabled for this project"),
        ));
    }
    let natural = SessionKind::default_for(harness.interact());
    let kind = input.kind.unwrap_or(natural);
    match kind {
        SessionKind::Action => {
            return Err(invalid(
                "/kind",
                "an action session has no implementation yet",
            ))
        }
        k if k != natural => {
            return Err(invalid(
                "/kind",
                format!(
                    "a `{}` session runs {}",
                    k.as_str(),
                    if k == SessionKind::Chat {
                        "an ACP agent"
                    } else {
                        "a terminal harness, not ACP"
                    }
                ),
            ))
        }
        _ => {}
    }
    let acp_agent = match (kind, input.acp_agent.clone()) {
        (SessionKind::Chat, Some(name)) => {
            if crate::acp::agents::find(adapters, config, &name).is_none() {
                return Err(invalid(
                    "/acp_agent",
                    format!("no ACP agent named `{name}`"),
                ));
            }
            Some(name)
        }
        (SessionKind::Chat, None) if input.harness.is_none() => Some(
            crate::acp::agents::default_agent(adapters, config)
                .ok_or_else(|| invalid("/acp_agent", "there are no ACP agents"))?,
        ),
        (SessionKind::Chat, None) => return Err(invalid("/acp_agent", "an ACP session needs one")),
        (_, Some(_)) => return Err(invalid("/acp_agent", "only an ACP session names one")),
        (_, None) => None,
    };
    Ok((key, kind, acp_agent))
}

/// `agent_session.open { thread, kind?, harness?, acp_agent?, title? }`:
/// a new slot on the thread. It only inserts the row — it never starts a
/// process and never sends a prompt. Undone by closing it.
pub fn open_op(
    config: Arc<RwLock<OxplowConfig>>,
    harnesses: HarnessRegistry,
    adapters: AcpAdapterRegistry,
) -> Op {
    Op::new(
        "agent_sessions.write",
        "open",
        schema::<OpenInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: OpenInput = parse(input)?;
            let thread = ref_id(&input.thread, "thread", "/thread")?;
            let row = oxplow_db::thread_store::get_tx(ctx.conn, thread)
                .map_err(sql)?
                .filter(|t| t.archived_at.is_none())
                .ok_or_else(|| invalid("/thread", format!("no thread `{}`", thread_ref(thread))))?;
            if row.status == ThreadStatus::Closed {
                return Err(invalid(
                    "/thread",
                    format!("`{}` is closed; reopen it first", thread_ref(thread)),
                ));
            }
            in_own_stream(ctx, thread)?;
            let config = crate::config_service::read_config(&config);
            let (harness, kind, acp_agent) = choose_agent(&config, &harnesses, &adapters, &input)?;
            let session = insert_tx(
                ctx.conn,
                &NewAgentSession {
                    thread_id: thread,
                    kind,
                    harness,
                    acp_agent,
                    title: input.title.unwrap_or_default(),
                },
                Timestamp::now(),
            )?;
            Ok(HandlerOutput {
                inverse: call(CLOSE, json!({ "session": agent_session_ref(session.id) })),
                ..result(&session)
            })
        })),
    )
}

/// `agent_session.rename { session, title }`; undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "agent_sessions.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            let before = set_title_tx(ctx.conn, session.id, &input.title, Timestamp::now())?
                .unwrap_or_default();
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            Ok(HandlerOutput {
                inverse: call(RENAME, json!({ "session": input.session, "title": before })),
                ..result(&after)
            })
        })),
    )
}

/// `agent_session.close { session }`: its slot closes, its open turns end
/// and it logs `stopped`, in one transaction; its process stops once that
/// commits. Destructive (a process ends), so a person confirms it and an
/// agent's waits as a proposal. Not undoable: a stopped process can't be.
pub fn close_op(processes: crate::agent_sessions::SessionProcesses) -> Op {
    Op::new(
        "agent_sessions.write",
        "close",
        schema::<SessionInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: SessionInput = parse(input)?;
            let session = load(ctx, &input.session)?;
            in_own_stream(ctx, session.thread_id)?;
            if !session.is_open() {
                return Ok(result(&session));
            }
            oxplow_db::agent_stores::close_session_tx(
                ctx.conn,
                &ctx.events,
                session.id,
                SessionCloseReason::Closed,
                Timestamp::now(),
            )?;
            let after = get_tx(ctx.conn, session.id)?
                .ok_or_else(|| invalid("/session", "the session vanished"))?;
            let processes = processes.clone();
            let id = session.id;
            Ok(HandlerOutput {
                after_commit: Some(Box::new(move || processes.kill(id))),
                ..result(&after)
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

/// The agent-session commands, for the bus.
pub fn ops(
    config: Arc<RwLock<OxplowConfig>>,
    harnesses: HarnessRegistry,
    adapters: AcpAdapterRegistry,
    processes: crate::agent_sessions::SessionProcesses,
) -> Vec<Op> {
    vec![
        open_op(config, harnesses, adapters),
        rename_op(),
        close_op(processes),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::AgentSessionStore as _;
    use oxplow_domain::Actor;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn run(
        fx: &EffortFixture,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
        confirmed: bool,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, confirmed).await
    }

    async fn open(fx: &EffortFixture, actor: &Actor, input: serde_json::Value) -> AgentSession {
        let out = run(fx, actor, OPEN, input, false).await.unwrap();
        serde_json::from_value(out.result).unwrap()
    }

    async fn get(fx: &EffortFixture, id: AgentSessionId) -> AgentSession {
        fx.svc.agent_session_store.get(&id).await.unwrap().unwrap()
    }

    /// Opening adds a slot of the default agent and starts nothing; undo
    /// closes it.
    #[tokio::test]
    async fn opening_adds_a_slot_and_undo_closes_it() {
        let fx = services_with_effort().await;
        let out = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread_ref(fx.thread), "title": "review" }),
            false,
        )
        .await
        .unwrap();
        let s: AgentSession = serde_json::from_value(out.result).unwrap();
        assert_eq!(
            (s.kind, s.harness, s.title.as_str()),
            (SessionKind::Terminal, "claude".to_string(), "review"),
            "the first declared harness, with no agents: named"
        );
        assert!(!fx
            .svc
            .terminal_sessions
            .session_id_for_key(&crate::terminal_sessions::agent_pane_key(s.id))
            .await
            .is_some());
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert!(!get(&fx, s.id).await.is_open());
    }

    /// The input is checked: an unknown field and a bad value name their
    /// field; an action session isn't built; a harness nothing registers,
    /// or the project hasn't enabled, or an ACP agent named wrong, is
    /// refused.
    #[tokio::test]
    async fn the_input_is_checked() {
        let fx = services_with_effort().await;
        let thread = thread_ref(fx.thread);
        fx.svc.config.write().unwrap().agents = vec!["claude".into()];
        for (input, field) in [
            (json!({ "thread": thread, "kind": "action" }), "/kind"),
            (json!({ "thread": thread, "kind": "chat" }), "/kind"),
            (json!({ "thread": thread, "harness": "codex" }), "/harness"),
            (json!({ "thread": thread, "harness": "nope" }), "/harness"),
            (
                json!({ "thread": thread, "acp_agent": "gemini" }),
                "/acp_agent",
            ),
            (json!({ "thread": "thread:thr99" }), "/thread"),
        ] {
            let err = run(&fx, &Actor::Human, OPEN, input.clone(), false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{input}: {err:?}"
            );
        }
        let err = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread, "agent": "claude" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("agent"), "{err}");
        fx.svc.config.write().unwrap().agents.push("acp".into());
        let err = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "thread": thread, "harness": "acp", "acp_agent": "nope" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/acp_agent"),
            "{err:?}"
        );
        let chat = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread, "harness": "acp", "acp_agent": "gemini" }),
        )
        .await;
        assert_eq!(
            (chat.kind, chat.acp_agent.as_deref()),
            (SessionKind::Chat, Some("gemini"))
        );
    }

    /// An agent opens and renames sessions only in its own stream, and its
    /// close waits for a person as a proposal.
    #[tokio::test]
    async fn an_agent_opens_in_its_own_stream_and_its_close_is_proposed() {
        let fx = services_with_effort().await;
        let mine = open(&fx, &agent(&fx), json!({ "thread": thread_ref(fx.thread) })).await;
        let err = run(
            &fx,
            &Actor::Agent {
                session_id: None,
                thread_id: Some(ThreadId::new(99)),
                stream_id: None,
            },
            OPEN,
            json!({ "thread": thread_ref(fx.thread) }),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let r = agent_session_ref(mine.id);
        let err = run(&fx, &agent(&fx), CLOSE, json!({ "session": r }), true)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        assert!(get(&fx, mine.id).await.is_open());
    }

    /// Closing is destructive: a person confirms it. It closes the slot
    /// and its open turn in one run, audited with its `command.executed`;
    /// renaming undoes to the old title.
    #[tokio::test]
    async fn closing_asks_first_and_ends_the_open_turn() {
        use oxplow_domain::stores::AgentTurnStore as _;
        let fx = services_with_effort().await;
        let s = fx.session;
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(fx.thread),
                stream_id: None,
                agent_session_id: Some(s),
                session_id: Some("h1".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
                tool: None,
            })
            .await
            .unwrap();
        let r = agent_session_ref(s);
        let err = run(&fx, &Actor::Human, CLOSE, json!({ "session": r }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
            "{err:?}"
        );
        let out = run(&fx, &Actor::Human, CLOSE, json!({ "session": r }), true)
            .await
            .unwrap();
        let closed = get(&fx, s).await;
        assert_eq!(closed.closed_reason, Some(SessionCloseReason::Closed));
        assert!(fx
            .svc
            .agent_turn_store
            .list_open(&fx.thread)
            .await
            .unwrap()
            .is_empty());
        let audit = fx
            .svc
            .commands
            .audit_store()
            .list_recent(1)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(Some(audit.id), out.audit_id);
        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "command.executed"
                && e.envelope.payload["audit_id"] == audit.id));
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "agent.status.changed"
                && e.envelope.anchors.agent_session_id == Some(s)
                && e.envelope.payload["state"] == "stopped"));
    }

    #[tokio::test]
    async fn renaming_undoes_to_the_old_title() {
        let fx = services_with_effort().await;
        let r = agent_session_ref(fx.session);
        let out = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "session": r, "title": "tests" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(get(&fx, fx.session).await.title, "tests");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert_eq!(get(&fx, fx.session).await.title, "");
    }

    /// Closing a thread closes its sessions (`thread_closed`).
    #[tokio::test]
    async fn closing_a_thread_closes_its_sessions() {
        let fx = services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let stream = fx
            .svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let second = crate::test_fixtures::new_thread(&fx.svc, stream, "t").await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread_ref(second.id) }),
        )
        .await;
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": thread_ref(second.id) }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            get(&fx, s.id).await.closed_reason,
            Some(SessionCloseReason::ThreadClosed)
        );
    }

    fn mint(fx: &EffortFixture, session: &AgentSession, stream: oxplow_domain::StreamId) -> String {
        fx.svc.session_auth.mint(crate::session_auth::Principal {
            session: session.id,
            thread: session.thread_id,
            stream,
            harness: session.harness.clone(),
        })
    }

    /// A closed session's process stops, and so does its bearer: nothing
    /// can post or call as it after.
    #[tokio::test]
    async fn closing_a_session_or_its_thread_retires_its_bearer() {
        let fx = services_with_effort().await;
        use oxplow_domain::stores::ThreadStore as _;
        let stream = fx
            .svc
            .thread_store
            .get(&fx.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let session = get(&fx, fx.session).await;
        let token = mint(&fx, &session, stream);
        run(
            &fx,
            &Actor::Human,
            CLOSE,
            json!({ "session": agent_session_ref(session.id) }),
            true,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());

        let second = crate::test_fixtures::new_thread(&fx.svc, stream, "t").await;
        let s = open(
            &fx,
            &Actor::Human,
            json!({ "thread": thread_ref(second.id) }),
        )
        .await;
        let token = mint(&fx, &s, stream);
        run(
            &fx,
            &Actor::Human,
            crate::commands::thread::CLOSE,
            json!({ "thread": thread_ref(second.id) }),
            false,
        )
        .await
        .unwrap();
        assert!(fx.svc.session_auth.authenticate(&token).is_none());
    }
}
