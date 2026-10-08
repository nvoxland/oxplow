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
//! or stream's); a person's in the thread the window shows. With no
//! window open, or one that doesn't answer in time, the run is refused.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::{Actor, CommandError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::commands::ops::Op;
use crate::commands::{Handler, HandlerOutput, Invocation};
use crate::events::{EventBus, OxplowEvent};

/// How long a call waits for the window's answer.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

type Answer = tokio::sync::oneshot::Sender<Result<Value, String>>;

/// The daemon's line to its window.
pub struct ClientHost {
    events: EventBus,
    /// The capabilities the open window hosts; `None` until a window
    /// says it's there.
    hosted: parking_lot::Mutex<Option<BTreeSet<String>>>,
    pending: parking_lot::Mutex<HashMap<String, Answer>>,
    timeout: Duration,
}

impl ClientHost {
    pub fn new(events: EventBus) -> Self {
        Self::with_timeout(events, ANSWER_TIMEOUT)
    }

    pub fn with_timeout(events: EventBus, timeout: Duration) -> Self {
        Self {
            events,
            hosted: parking_lot::Mutex::default(),
            pending: parking_lot::Mutex::default(),
            timeout,
        }
    }

    /// The window is open and hosts `capabilities` (it says so when it
    /// starts, and again when it reconnects).
    pub fn register(&self, capabilities: Vec<String>) {
        *self.hosted.lock() = Some(capabilities.into_iter().collect());
    }

    /// The window answers call `id`: its result, or why it couldn't.
    /// `false` when no call is waiting under `id` (it timed out).
    pub fn answer(&self, id: &str, answer: Result<Value, String>) -> bool {
        match self.pending.lock().remove(id) {
            Some(waiting) => waiting.send(answer).is_ok(),
            None => false,
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
        match &*self.hosted.lock() {
            None => return Err(refused("no window is open".into())),
            Some(hosted) if !hosted.contains(capability) => {
                return Err(refused(format!("the window doesn't host `{capability}`")))
            }
            Some(_) => {}
        }
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
        self.pending.lock().insert(id.clone(), tx);
        self.events.emit(OxplowEvent::ClientCall {
            id: id.clone(),
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
            Ok(Err(_)) | Err(_) => Err(refused("the window didn't answer".into())),
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

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// The window's operations, run through `host` when the daemon runs them.
pub fn ops(host: &Arc<ClientHost>) -> Vec<Op> {
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
    ops.push(ClientHost::op(
        host,
        "window.show",
        "find",
        schema::<NoInput>(),
    ));
    ops.push(ClientHost::op(
        host,
        "window.show",
        "quick_open",
        schema::<NoInput>(),
    ));
    ops.push(ClientHost::op(
        host,
        "agent_input.write",
        "draft",
        schema::<DraftInput>(),
    ));
    // The shell's, reached through the window.
    ops.push(ClientHost::op(
        host,
        "projects.write",
        "create",
        schema::<NoInput>(),
    ));
    ops.push(ClientHost::op(
        host,
        "projects.write",
        "open",
        schema::<OpenProjectInput>(),
    ));
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
        host.register(vec!["tabs.write".into()]);
        let window = {
            let host = host.clone();
            tokio::spawn(async move {
                let OxplowEvent::ClientCall {
                    id,
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
                assert!(host.answer(&id, Ok(json!({ "opened": input.0["ref"] }))));
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
        fx.svc.client_host.register(vec!["tabs.write".into()]);
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
                        host.answer(&id, Ok(json!({ "open": true }))),
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
        host.register(vec!["editor.write".into()]);
        assert!(call(agent(Some(1)))
            .await
            .contains("doesn't host `tabs.write`"));
        host.register(vec!["tabs.write".into()]);
        assert!(call(agent(Some(1))).await.contains("didn't answer"));
        assert!(call(agent(None)).await.contains("this agent has none"));
        assert!(!host.answer("gone", Ok(json!(null))));
    }
}
