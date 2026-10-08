//! The window as a command host (`.context/commands.md` "Where a command
//! runs"): a command backed by a capability the window hosts
//! (`Host::Window`: `tabs.write`, …) runs in the window — and one the
//! app shell hosts (`Host::Shell`: `projects.write`), which the window
//! reaches. The window's own
//! runs never come here; a run on the daemon — an agent's, a script's —
//! reaches the window through [`ClientHost::call`]: an
//! `OxplowEvent::ClientCall` to the project's one window, answered with
//! `answer_client_call`, which the command waits for.
//!
//! An agent's call acts in its own thread's tabs (never another thread's
//! or stream's); a person's in the thread the window shows. A call goes to
//! one window — the last registered that hosts it — and only that window
//! answers. With no window open, one that closed (`unregister`) or one
//! that doesn't answer in time (then forgotten until it registers again),
//! the run is refused. What the window runs on the daemon to answer a call
//! (Save's `oxplow.file.save`) runs as the call's actor ([`ClientHost::caller`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::{Actor, CommandError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::commands::ops::Op;
use crate::commands::util::schema;
use crate::commands::{Handler, HandlerOutput, Invocation};
use crate::events::{EventBus, OxplowEvent};

/// How long a call waits for the window's answer.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

type Answer = tokio::sync::oneshot::Sender<Result<Value, String>>;

/// An open window: its id (minted by the window when it starts) and the
/// capabilities it hosts.
struct Window {
    client: String,
    hosted: BTreeSet<String>,
}

/// A call waiting for its window: who ran it, and where the answer goes.
struct Pending {
    client: String,
    actor: Actor,
    answer: Answer,
}

/// The daemon's line to its window.
pub struct ClientHost {
    events: EventBus,
    /// The open windows, the most recently registered last: a call goes
    /// to the last one that hosts its capability.
    windows: parking_lot::Mutex<Vec<Window>>,
    pending: parking_lot::Mutex<HashMap<String, Pending>>,
    timeout: Duration,
}

impl ClientHost {
    pub fn new(events: EventBus) -> Self {
        Self::with_timeout(events, ANSWER_TIMEOUT)
    }

    pub fn with_timeout(events: EventBus, timeout: Duration) -> Self {
        Self {
            events,
            windows: parking_lot::Mutex::default(),
            pending: parking_lot::Mutex::default(),
            timeout,
        }
    }

    /// Window `client` is open and hosts `capabilities` (it says so when
    /// it starts, and again when it reconnects). It takes the calls from
    /// now on.
    pub fn register(&self, client: &str, capabilities: Vec<String>) {
        let mut windows = self.windows.lock();
        windows.retain(|w| w.client != client);
        windows.push(Window {
            client: client.into(),
            hosted: capabilities.into_iter().collect(),
        });
    }

    /// Window `client` closed: it takes no more calls, and the ones it
    /// hadn't answered are refused now rather than when they time out.
    pub fn unregister(&self, client: &str) {
        self.windows.lock().retain(|w| w.client != client);
        // Dropping a waiting call's sender answers it "closed".
        self.pending.lock().retain(|_, p| p.client != client);
    }

    /// Window `client` answers call `id`: its result, or why it couldn't.
    /// `false` when no call of that window's is waiting under `id` (it
    /// timed out, or went to another window).
    pub fn answer(&self, client: &str, id: &str, answer: Result<Value, String>) -> bool {
        let mut pending = self.pending.lock();
        if pending.get(id).is_none_or(|p| p.client != client) {
            return false;
        }
        let waiting = pending.remove(id).expect("checked above");
        waiting.answer.send(answer).is_ok()
    }

    /// Who ran call `id`, while it waits for window `client`'s answer: a
    /// command the window runs to answer it runs as them
    /// (`run_command_for_call`), never as the person at the window.
    pub fn caller(&self, client: &str, id: &str) -> Result<Actor, CommandError> {
        match self.pending.lock().get(id) {
            Some(p) if p.client == client => Ok(p.actor.clone()),
            _ => Err(CommandError::Invalid {
                field: Some("/call".into()),
                message: format!("no call `{id}` is waiting for this window"),
            }),
        }
    }

    /// Have the window do `capability`'s `op` with `input` for `actor`.
    pub async fn call(
        &self,
        actor: &Actor,
        capability: &str,
        op: &str,
        input: Value,
    ) -> Result<Value, CommandError> {
        let refused = |message: String| CommandError::Unavailable {
            message,
            retry_after_ms: None,
        };
        let client = {
            let windows = self.windows.lock();
            if windows.is_empty() {
                return Err(refused("no window is open".into()));
            }
            match windows.iter().rev().find(|w| w.hosted.contains(capability)) {
                Some(w) => w.client.clone(),
                None => return Err(refused(format!("the window doesn't host `{capability}`"))),
            }
        };
        let thread_id = match actor.agent_thread() {
            // An agent acts in its own thread; one without a thread has no
            // tabs to act in.
            Some(None) => {
                return Err(CommandError::Invalid {
                    field: None,
                    message: format!("`{capability}` acts in a thread's tabs; this agent has none"),
                })
            }
            Some(thread) => thread,
            None => None,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().insert(
            id.clone(),
            Pending {
                client: client.clone(),
                actor: actor.clone(),
                answer: tx,
            },
        );
        self.events.emit(OxplowEvent::ClientCall {
            id: id.clone(),
            client: client.clone(),
            thread_id,
            actor: actor.source(),
            capability: capability.into(),
            op: op.into(),
            input: oxplow_domain::Json(input),
        });
        let answered = tokio::time::timeout(self.timeout, rx).await;
        self.pending.lock().remove(&id);
        match answered {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(message))) => Err(CommandError::Failed { message }),
            Ok(Err(_)) => Err(refused("the window closed before it answered".into())),
            Err(_) => {
                // A window that doesn't answer is taken as gone until it
                // registers again, so the next call is refused at once
                // instead of waiting out the timeout too.
                self.windows.lock().retain(|w| w.client != client);
                Err(refused("the window didn't answer".into()))
            }
        }
    }

    /// `capability`'s operation `op` on the daemon: its runs go to the
    /// window.
    pub fn op(
        host: &Arc<ClientHost>,
        capability: &'static str,
        op: &'static str,
        input_schema: Value,
    ) -> Op {
        let host = host.clone();
        Op::new(
            capability,
            op,
            input_schema,
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
                let host = host.clone();
                Box::pin(async move {
                    let result = host.call(&invocation.actor, capability, op, input).await?;
                    Ok(HandlerOutput {
                        result,
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
    }
}

/// `tabs.write`'s operations' input: the tab (its id, `file:src/a.rs`,
/// `page:git-dashboard`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabInput {
    /// The tab's id: a page's ref (`file:src/a.rs`, `work_item:oxplow:tsk3`,
    /// `page:git-dashboard`).
    #[serde(rename = "ref")]
    pub tab: String,
}

/// `editor.write` `save`'s input: the file (its tab id, `file:src/a.rs`);
/// none saves the one the thread shows.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    #[serde(rename = "ref", default)]
    pub tab: Option<String>,
}

/// `agent_input.write` `draft`'s input: the text to put in the agent's
/// input, unsent.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    pub text: String,
}

/// `projects.write` `open`'s input: the folder (none asks the person to
/// pick one), and whether it opens in a new window.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenProjectInput {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub new_window: bool,
}

/// An operation that takes nothing.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

/// The window's operations, run through `host` when the daemon runs them.
pub fn ops(host: &Arc<ClientHost>) -> Vec<Op> {
    use oxplow_domain::Invokers;
    // An agent's in its own thread; a person's anywhere.
    let mut ops: Vec<Op> = ["open", "close", "focus"]
        .into_iter()
        .map(|op| ClientHost::op(host, "tabs.write", op, schema::<TabInput>()))
        .collect();
    ops.push(ClientHost::op(
        host,
        "editor.write",
        "save",
        schema::<SaveInput>(),
    ));
    // The window's own chrome: a person's.
    ops.push(
        ClientHost::op(host, "window.show", "find", schema::<NoInput>())
            .open_to(Invokers::HUMAN_ONLY),
    );
    ops.push(
        ClientHost::op(host, "window.show", "quick_open", schema::<NoInput>())
            .open_to(Invokers::HUMAN_ONLY),
    );
    // A person's, through a lens too: oxplow never types for the agent.
    ops.push(
        ClientHost::op(host, "agent_input.write", "draft", schema::<DraftInput>())
            .open_to(Invokers::NO_AGENT),
    );
    // The shell's, reached through the window: a person's.
    ops.push(
        ClientHost::op(host, "projects.write", "create", schema::<NoInput>())
            .open_to(Invokers::HUMAN_ONLY),
    );
    ops.push(
        ClientHost::op(host, "projects.write", "open", schema::<OpenProjectInput>())
            .open_to(Invokers::HUMAN_ONLY),
    );
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ThreadId;
    use serde_json::json;

    fn agent(thread: Option<i64>) -> Actor {
        Actor::Agent {
            thread_id: thread.map(ThreadId::new),
            stream_id: None,
        }
    }

    /// The window answers a call it got as an event; the call carries the
    /// agent's own thread.
    #[tokio::test]
    async fn a_call_goes_to_the_window_with_the_agents_thread() {
        let events = EventBus::new();
        let mut seen = events.subscribe_ui();
        let host = Arc::new(ClientHost::new(events));
        host.register("w1", vec!["tabs.write".into()]);
        let window = {
            let host = host.clone();
            tokio::spawn(async move {
                let OxplowEvent::ClientCall {
                    id,
                    client,
                    thread_id,
                    actor,
                    capability,
                    op,
                    input,
                } = seen.recv().await.unwrap()
                else {
                    panic!("a client call")
                };
                assert_eq!(
                    (thread_id, actor.as_str(), capability.as_str(), op.as_str()),
                    (Some(ThreadId::new(3)), "agent:thr3", "tabs.write", "open")
                );
                assert_eq!(client, "w1");
                assert!(host.answer("w1", &id, Ok(json!({ "opened": input.0["ref"] }))));
            })
        };
        let out = host
            .call(
                &agent(Some(3)),
                "tabs.write",
                "open",
                json!({ "ref": "file:a.rs" }),
            )
            .await
            .unwrap();
        assert_eq!(out, json!({ "opened": "file:a.rs" }));
        window.await.unwrap();
    }

    /// An agent runs `oxplow.tab.open` (declared in oxplow-foundation over
    /// the window's `tabs.write`): it reaches the window, in the agent's
    /// thread, and the window's answer is the run's result. A view isn't
    /// recorded.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_opens_a_tab_in_its_own_thread() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let spec = fx.svc.commands.spec("oxplow.tab.open").unwrap();
        assert_eq!(
            spec.op,
            Some(oxplow_domain::OpRef {
                capability: "tabs.write".into(),
                op: "open".into()
            })
        );
        assert_eq!(spec.effect, oxplow_domain::CommandEffect::Read);
        let mut seen = fx.svc.events.subscribe_ui();
        fx.svc.client_host.register("w1", vec!["tabs.write".into()]);
        let host = fx.svc.client_host.clone();
        let window = tokio::spawn(async move {
            loop {
                if let OxplowEvent::ClientCall {
                    id,
                    thread_id,
                    input,
                    ..
                } = seen.recv().await.unwrap()
                {
                    return (
                        thread_id,
                        input.0,
                        host.answer("w1", &id, Ok(json!({ "open": true }))),
                    );
                }
            }
        });
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                "oxplow.tab.open",
                json!({ "ref": "file:src/a.rs" }),
                false,
            )
            .await
            .unwrap();
        let (thread, input, answered) = window.await.unwrap();
        assert_eq!(thread, Some(fx.thread));
        assert_eq!(input, json!({ "ref": "file:src/a.rs" }));
        assert!(answered);
        assert_eq!(out.result, json!({ "open": true }));
        assert_eq!(out.audit_id, None);
    }

    /// No window, one that doesn't host it, one that doesn't answer, an
    /// agent with no thread: each is refused, saying why.
    #[tokio::test]
    async fn a_call_nothing_can_answer_is_refused() {
        let events = EventBus::new();
        let host = ClientHost::with_timeout(events, Duration::from_millis(50));
        let call = |actor| {
            let host = &host;
            async move {
                host.call(&actor, "tabs.write", "open", json!({}))
                    .await
                    .unwrap_err()
                    .to_string()
            }
        };
        assert!(call(agent(Some(1))).await.contains("no window is open"));
        host.register("w1", vec!["editor.write".into()]);
        assert!(call(agent(Some(1)))
            .await
            .contains("doesn't host `tabs.write`"));
        host.register("w1", vec!["tabs.write".into()]);
        assert!(call(agent(None)).await.contains("this agent has none"));
        assert!(call(agent(Some(1))).await.contains("didn't answer"));
        // Taken as gone: the next call is refused at once, not after
        // another timeout.
        assert!(call(agent(Some(1))).await.contains("no window is open"));
        assert!(!host.answer("w1", "gone", Ok(json!(null))));
    }

    /// The window that registered last takes the calls, and only it may
    /// answer one; the call's actor is readable while it waits, by that
    /// window only.
    #[tokio::test]
    async fn a_call_goes_to_one_window_and_only_it_answers() {
        let events = EventBus::new();
        let mut seen = events.subscribe_ui();
        let host = Arc::new(ClientHost::new(events));
        host.register("w1", vec!["tabs.write".into()]);
        host.register("w2", vec!["tabs.write".into()]);
        let window = {
            let host = host.clone();
            tokio::spawn(async move {
                let OxplowEvent::ClientCall { id, client, .. } = seen.recv().await.unwrap() else {
                    panic!("a client call")
                };
                assert_eq!(client, "w2");
                assert_eq!(host.caller("w2", &id).unwrap(), agent(Some(3)));
                assert!(host.caller("w1", &id).is_err(), "another window's call");
                assert!(!host.answer("w1", &id, Ok(json!("w1"))));
                assert!(host.answer("w2", &id, Ok(json!("w2"))));
                assert!(host.caller("w2", &id).is_err(), "answered");
            })
        };
        let out = host
            .call(&agent(Some(3)), "tabs.write", "open", json!({}))
            .await
            .unwrap();
        assert_eq!(out, json!("w2"));
        window.await.unwrap();
    }

    /// A window that closes takes no more calls, and the one it hadn't
    /// answered is refused then, not when it would have timed out.
    #[tokio::test]
    async fn a_closed_windows_calls_are_refused_at_once() {
        let events = EventBus::new();
        let mut seen = events.subscribe_ui();
        let host = Arc::new(ClientHost::new(events));
        host.register("w1", vec!["tabs.write".into()]);
        let closer = {
            let host = host.clone();
            tokio::spawn(async move {
                let _ = seen.recv().await.unwrap();
                host.unregister("w1");
            })
        };
        let started = std::time::Instant::now();
        let err = host
            .call(&agent(Some(3)), "tabs.write", "open", json!({}))
            .await
            .unwrap_err()
            .to_string();
        closer.await.unwrap();
        assert!(err.contains("closed before it answered"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let err = host
            .call(&agent(Some(3)), "tabs.write", "open", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no window is open"), "{err}");
    }
}
