//! One Assistant session: its transcript, the Rig history it sends, and
//! the turn running on it. A turn is a task driving Rig's stream; its
//! hook ([`TurnHook`]) writes what happens into the transcript as it
//! happens (text and thoughts as they stream, each tool call's status)
//! and holds a change for the person's answer on its approval card.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::StreamExt;
use oxplow_ai_rig::rig_core::effect::EffectId;
use oxplow_ai_rig::rig_core::message::Message;
use rig_agent::agent::{
    DispatchAction, DispatchEvent, MultiTurnStreamItem, ObservationAction, OutcomeAction,
    OutcomeEvent, ReasoningDelta, TextDelta,
};
use rig_agent::prelude::{Agent, AgentBuilder, AgentHook, HookContext};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use oxplow_domain::run::{Mode, RunContext};
use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
use oxplow_domain::{Actor, StreamId, ThreadId};

use crate::ai_service::{AiServiceError, BuiltinRole, Role, RoleBinding};
use crate::chat::model::{
    ChatUpdate, PermissionAnswer, PermissionKind, PermissionOption, ToolCall, ToolKind, ToolStatus,
};
use crate::chat::transcript::{ItemBody, Transcript};
use crate::commands::CommandBus;
use crate::events::{EventBus, OxplowEvent};
use crate::Services;

use super::prompt::{self, Place};
use super::tools::{self, AuditRef, ToolSpec};
use super::{
    AssistantContext, AssistantError, AssistantModel, AssistantModelChoice, AssistantModelOption,
    AssistantProblem, AssistantProblemKind, AssistantSnapshot, AssistantStatus,
};

/// Model calls one turn may make (Rig's default is one).
const MAX_TURNS: usize = 16;

/// What the model is told when the person declines a call.
pub const DECLINED: &str = "the person declined";

/// Where the session works: its stream's title, branch and worktree, and
/// its thread's title.
pub(super) async fn place(
    svc: &Services,
    stream: StreamId,
    thread: Option<ThreadId>,
) -> Result<Place, AssistantError> {
    let found = svc
        .stream_store
        .get(&stream)
        .await
        .map_err(|e| AssistantError::Failed(e.to_string()))?
        .ok_or_else(|| AssistantError::Invalid(format!("there is no stream `{stream}`")))?;
    let thread = match thread {
        Some(id) => {
            let t = svc
                .thread_store
                .get(&id)
                .await
                .map_err(|e| AssistantError::Failed(e.to_string()))?
                .ok_or_else(|| AssistantError::Invalid(format!("there is no thread `{id}`")))?;
            Some((id.to_string(), t.title))
        }
        None => None,
    };
    Ok(Place {
        stream: stream.to_string(),
        stream_title: found.title,
        branch: found.branch,
        worktree: found.worktree_path,
        thread,
    })
}

pub(super) struct Session {
    id: String,
    thread: Option<ThreadId>,
    context: Option<AssistantContext>,
    place: Place,
    events: EventBus,
    state: Mutex<State>,
}

struct State {
    transcript: Transcript,
    /// What Rig is sent before each prompt: every finished turn's messages.
    history: Vec<Message>,
    allow_changes: bool,
    /// The person's pick; `None` runs on the role's binding.
    choice: Option<RoleBinding>,
    /// The role's binding, as last read.
    role_model: Option<RoleBinding>,
    models: Vec<RoleBinding>,
    problem: Option<AssistantProblem>,
    /// Open approval cards: their answer, and their item.
    asks: HashMap<String, (oneshot::Sender<bool>, u64)>,
    next_ask: u64,
    next_call: u64,
    /// The running turn: its number and its stop.
    turn: Option<(u64, oneshot::Sender<()>)>,
    /// The number the last turn started with.
    turns: u64,
    closed: bool,
}

/// How a turn ended.
enum Ended {
    Answered(Vec<Message>),
    Failed(String),
    Stopped,
}

impl Session {
    pub(super) fn new(
        id: String,
        _stream: StreamId,
        thread: Option<ThreadId>,
        context: Option<AssistantContext>,
        place: Place,
        events: EventBus,
    ) -> Self {
        Self {
            id,
            thread,
            context,
            place,
            events,
            state: Mutex::new(State {
                transcript: Transcript::default(),
                history: Vec::new(),
                allow_changes: false,
                choice: None,
                role_model: None,
                models: Vec::new(),
                problem: None,
                asks: HashMap::new(),
                next_ask: 0,
                next_call: 0,
                turn: None,
                turns: 0,
                closed: false,
            }),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut state)
    }

    /// Tell the panel the session changed (its newest `seq`).
    fn changed(&self) {
        let seq = self.with(|s| s.transcript.head_seq());
        self.events.emit(OxplowEvent::AssistantChanged {
            id: self.id.clone(),
            seq,
        });
    }

    /// Re-read the role's binding and the models to pick from.
    pub(super) async fn refresh(&self, svc: &Services) {
        let role = svc.ai.binding_for(&Role::ASSISTANT);
        let models = svc
            .ai
            .model_choices(BuiltinRole::Assistant)
            .await
            .unwrap_or_default();
        self.with(|s| {
            s.models = models;
            match role {
                Ok(binding) => {
                    s.role_model = Some(binding);
                    if s.problem
                        .as_ref()
                        .is_some_and(|p| p.kind == AssistantProblemKind::NoModel)
                    {
                        s.problem = None;
                    }
                }
                Err(e) => {
                    s.role_model = None;
                    s.problem = (s.choice.is_none()).then(|| no_model(&e));
                }
            }
        });
    }

    pub(super) fn snapshot(&self, since: u64) -> AssistantSnapshot {
        self.with(|s| AssistantSnapshot {
            status: if s.turn.is_some() {
                AssistantStatus::Running
            } else {
                AssistantStatus::Idle
            },
            items: s.transcript.since(since),
            seq: s.transcript.head_seq(),
            model: s
                .choice
                .as_ref()
                .or(s.role_model.as_ref())
                .map(|b| AssistantModel {
                    provider: b.provider.clone(),
                    model: b.model.clone(),
                }),
            models: s
                .models
                .iter()
                .map(|b| AssistantModelOption {
                    provider: b.provider.clone(),
                    model: b.model.clone(),
                    label: format!("{} · {}", b.provider, b.model),
                })
                .collect(),
            allow_changes: s.allow_changes,
            problem: s.problem.clone(),
        })
    }

    pub(super) async fn set(
        &self,
        svc: &Services,
        model: Option<AssistantModelChoice>,
        allow_changes: Option<bool>,
    ) {
        self.with(|s| {
            if let Some(allow) = allow_changes {
                s.allow_changes = allow;
            }
            match model {
                Some(AssistantModelChoice::Default(_)) => s.choice = None,
                Some(AssistantModelChoice::Model(m)) => {
                    s.choice = Some(RoleBinding::new(m.provider, m.model));
                    s.problem = None;
                }
                None => {}
            }
        });
        self.refresh(svc).await;
        self.changed();
    }

    pub(super) async fn prompt(
        self: &Arc<Self>,
        svc: &Services,
        text: &str,
    ) -> Result<(), AssistantError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AssistantError::Invalid("say what you want to ask".into()));
        }
        let (choice, allow_changes) = self.with(|s| {
            if s.closed {
                return Err(AssistantError::NotFound(self.id.clone()));
            }
            if s.turn.is_some() {
                return Err(AssistantError::TurnInFlight);
            }
            Ok((s.choice.clone(), s.allow_changes))
        })?;
        let model = match svc
            .ai
            .agent_model(BuiltinRole::Assistant, choice.as_ref())
            .await
        {
            Ok(model) => model,
            Err(e) => {
                let problem = if unbound(&e) {
                    no_model(&e)
                } else {
                    AssistantProblem {
                        kind: AssistantProblemKind::Model,
                        message: e.to_string(),
                    }
                };
                let message = problem.message.clone();
                self.with(|s| s.problem = Some(problem));
                self.changed();
                return Err(if unbound(&e) {
                    AssistantError::NoModel(message)
                } else {
                    AssistantError::Failed(message)
                });
            }
        };
        let via = format!("assistant:{}", self.id);
        let sql = !model.binding.forbids(oxplow_domain::ai::AiRestriction::Sql);
        let offered = tools::tools(&svc.commands, &via, allow_changes, sql);
        let hook = TurnHook {
            session: self.clone(),
            bus: svc.commands.clone(),
            ai: svc.ai.clone(),
            binding: model.binding.clone(),
            asked: Mutex::new(HashMap::new()),
            specs: offered.specs.clone(),
            calls: Mutex::new(HashMap::new()),
        };
        let agent = AgentBuilder::new(model.handle)
            .preamble(prompt::system(
                &self.place,
                self.context.as_ref(),
                allow_changes,
            ))
            .default_max_turns(MAX_TURNS)
            .dynamic_tools(offered.tools)
            .add_hook(hook)
            .build();
        let (stop, stopped) = oneshot::channel();
        let (turn, history) = self.with(|s| {
            if s.problem
                .as_ref()
                .is_some_and(|p| p.kind == AssistantProblemKind::Model)
            {
                s.problem = None;
            }
            s.transcript.push(ItemBody::User {
                text: text.to_string(),
                context: None,
            });
            s.turns += 1;
            s.turn = Some((s.turns, stop));
            (s.turns, s.history.clone())
        });
        self.changed();
        let session = self.clone();
        let text = text.to_string();
        tokio::spawn(async move {
            let ended = run_turn(&agent, &text, history, stopped).await;
            session.finish(Some(turn), ended);
        });
        Ok(())
    }

    /// A turn is over: keep what it answered, say why it failed, and close
    /// what it left open (its cards, its unfinished calls).
    ///
    /// `turn` is the turn that ended on its own; a stopped turn is settled
    /// by [`Session::cancel`] (`None`) at once, and its task ending later
    /// finds its turn gone and leaves the next turn alone.
    fn finish(&self, turn: Option<u64>, ended: Ended) {
        let settled = self.with(|s| {
            if let Some(turn) = turn {
                if s.turn.as_ref().map(|t| t.0) != Some(turn) {
                    return false;
                }
            }
            s.turn = None;
            for (_, (_, item)) in s.asks.drain() {
                s.transcript.update(item, |b| {
                    if let ItemBody::Permission { answer, .. } = b {
                        answer.get_or_insert(PermissionAnswer::Cancelled);
                    }
                });
            }
            let unfinished: Vec<u64> = s
                .transcript
                .since(0)
                .into_iter()
                .rev()
                .take_while(|i| !matches!(i.body, ItemBody::User { .. }))
                .filter(|i| {
                    matches!(&i.body, ItemBody::Tool { call }
                        if matches!(call.status, ToolStatus::Pending | ToolStatus::InProgress))
                })
                .map(|i| i.id)
                .collect();
            for id in unfinished {
                s.transcript.update(id, |b| {
                    if let ItemBody::Tool { call } = b {
                        call.status = ToolStatus::Failed;
                        call.text.push("Stopped".into());
                    }
                });
            }
            match ended {
                Ended::Answered(messages) => s.history.extend(messages),
                Ended::Failed(message) => {
                    s.transcript.push(ItemBody::Error { message });
                }
                // A stopped turn keeps what the transcript shows; the
                // model's history gets nothing of it.
                Ended::Stopped => {}
            }
            true
        });
        if settled {
            self.changed();
        }
    }

    pub(super) fn cancel(&self) {
        if let Some((_, stop)) = self.with(|s| s.turn.take()) {
            let _ = stop.send(());
            self.finish(None, Ended::Stopped);
        }
    }

    pub(super) fn close(&self) {
        self.with(|s| s.closed = true);
        self.cancel();
    }

    pub(super) fn answer(&self, ask: &str, allow: bool) -> Result<(), AssistantError> {
        let found = self.with(|s| {
            let (answer, item) = s.asks.remove(ask)?;
            s.transcript.update(item, |b| {
                if let ItemBody::Permission { answer, .. } = b {
                    *answer = Some(PermissionAnswer::Selected {
                        option_id: if allow { "allow" } else { "decline" }.into(),
                    });
                }
            });
            Some(answer)
        });
        let answer = found.ok_or_else(|| {
            AssistantError::Invalid(format!("`{ask}` isn't a card waiting for an answer"))
        })?;
        let _ = answer.send(allow);
        self.changed();
        Ok(())
    }

    pub(super) async fn handoff(
        &self,
        svc: &Services,
        to_thread: &str,
    ) -> Result<Value, AssistantError> {
        let from = self.thread.ok_or_else(|| {
            AssistantError::Invalid("this Assistant session has no thread to hand off from".into())
        })?;
        let body =
            self.with(|s| prompt::handoff_body(self.context.as_ref(), &s.transcript.since(0)));
        let run = RunContext::core(Actor::Human, Mode::External)
            .acting_through(format!("assistant:{}", self.id));
        let outcome = svc
            .commands
            .run_in(
                &run,
                crate::commands::handoff::SEND,
                json!({ "from_thread": from.to_string(), "to_thread": to_thread, "body": body }),
                false,
            )
            .await
            .map_err(|e| AssistantError::Failed(e.to_string()))?;
        Ok(outcome.result)
    }

    // ---- what the turn's hook writes ----

    fn chunk(&self, update: ChatUpdate) {
        self.with(|s| s.transcript.apply(update));
        self.changed();
    }

    /// A call is about to run: its item, and — for a change — its card
    /// and where its answer comes.
    fn call_started(&self, call: Started) -> (u64, Option<oneshot::Receiver<bool>>) {
        let Started {
            title,
            command,
            changes,
            args,
        } = call;
        let out = self.with(|s| {
            s.next_call += 1;
            let call_id = format!("call{}", s.next_call);
            let item = s.transcript.push(ItemBody::Tool {
                call: ToolCall {
                    id: call_id.clone(),
                    title: title.clone(),
                    name: Some(command.clone()),
                    kind: if changes {
                        ToolKind::Edit
                    } else {
                        ToolKind::Read
                    },
                    status: if changes {
                        ToolStatus::Pending
                    } else {
                        ToolStatus::InProgress
                    },
                    raw_input: Some(args),
                    ..ToolCall::default()
                },
            });
            if !changes {
                return (item.id, None);
            }
            s.next_ask += 1;
            let ask = format!("ask{}", s.next_ask);
            let card = s.transcript.push(ItemBody::Permission {
                request_id: ask.clone(),
                tool_call_id: call_id,
                title: format!("Run `{command}`: {title}"),
                options: vec![
                    PermissionOption {
                        id: "allow".into(),
                        name: "Allow".into(),
                        kind: PermissionKind::AllowOnce,
                    },
                    PermissionOption {
                        id: "decline".into(),
                        name: "Decline".into(),
                        kind: PermissionKind::RejectOnce,
                    },
                ],
                answer: None,
            });
            let (tx, rx) = oneshot::channel();
            s.asks.insert(ask, (tx, card.id));
            (item.id, Some(rx))
        });
        self.changed();
        out
    }

    fn call_update(&self, item: u64, f: impl FnOnce(&mut ToolCall)) {
        self.with(|s| {
            s.transcript.update(item, |b| {
                if let ItemBody::Tool { call } = b {
                    f(call);
                }
            })
        });
        self.changed();
    }
}

/// A call about to run, as its item shows it.
struct Started {
    title: String,
    /// The command it runs (for [`tools::RUN_COMMAND`], the one it names).
    command: String,
    changes: bool,
    /// Its input.
    args: Value,
}

impl Started {
    fn of(name: &str, spec: Option<&ToolSpec>, args: Value, bus: &CommandBus) -> Self {
        match spec {
            Some(ToolSpec::Read { command, summary }) => Started {
                title: summary.clone(),
                command: command.clone(),
                changes: false,
                args,
            },
            Some(ToolSpec::Find) => Started {
                title: "Find a command".into(),
                command: name.to_string(),
                changes: false,
                args,
            },
            Some(ToolSpec::Run) => {
                let command = args["command"].as_str().unwrap_or_default().to_string();
                let title = bus
                    .spec(&command)
                    .map(|s| s.summary)
                    .unwrap_or_else(|| command.clone());
                Started {
                    title,
                    command,
                    changes: true,
                    args: args["input"].clone(),
                }
            }
            None => Started {
                title: name.to_string(),
                command: name.to_string(),
                changes: false,
                args,
            },
        }
    }
}

/// Drive one turn until it answers, fails or is stopped. Stopping drops
/// the stream: the model's connection with it, and any call it awaits.
async fn run_turn(
    agent: &Agent,
    text: &str,
    history: Vec<Message>,
    mut stopped: oneshot::Receiver<()>,
) -> Ended {
    let mut stream = agent.prompt(text).history(history).stream();
    loop {
        tokio::select! {
            item = stream.next() => match item {
                Some(Ok(MultiTurnStreamItem::FinalResponse(response))) => {
                    return Ended::Answered(response.messages);
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Ended::Failed(e.to_string()),
                None => return Ended::Failed("the model stopped before it answered".into()),
            },
            _ = &mut stopped => return Ended::Stopped,
        }
    }
}

/// Whether `e` says no model is bound.
fn unbound(e: &AiServiceError) -> bool {
    matches!(
        e,
        AiServiceError::NotConfigured(_) | AiServiceError::RoleNotConfigured { .. }
    )
}

fn no_model(e: &AiServiceError) -> AssistantProblem {
    AssistantProblem {
        kind: AssistantProblemKind::NoModel,
        message: if unbound(e) {
            "No model is assigned to the Assistant. Assign one in Settings → AI (the \
             Assistant role, or Main, which it falls back to)."
                .into()
        } else {
            e.to_string()
        },
    }
}

/// One turn's hook: the transcript as the turn happens, and the approval
/// hold on a change.
struct TurnHook {
    session: Arc<Session>,
    bus: Arc<CommandBus>,
    ai: Arc<crate::ai_service::AiService>,
    binding: RoleBinding,
    /// Model calls under way: when each began, and what it was sent.
    asked: Mutex<HashMap<EffectId, (Instant, Value)>>,
    specs: Arc<std::collections::BTreeMap<String, ToolSpec>>,
    /// Rig's call ids → their tool items.
    calls: Mutex<HashMap<String, u64>>,
}

impl TurnHook {
    /// Record a model call's outcome in `ai_call`, as the Assistant's.
    async fn record(&self, event: &OutcomeEvent<'_>, started: Instant, request: Value) {
        let caller = format!("assistant:{}", self.session.id);
        let count = |n: Option<u64>| n.and_then(|n| i64::try_from(n).ok()).unwrap_or(0);
        let failure;
        let outcome = match event.outcome {
            Ok(_) => match event.completion() {
                Some(response) => Ok((
                    count(response.usage.input_tokens),
                    count(response.usage.output_tokens),
                    json!({ "choice": response.choice }),
                )),
                None => Ok((0, 0, Value::Null)),
            },
            Err(report) => {
                failure = report.to_string();
                Err(failure.as_str())
            }
        };
        self.ai
            .record_agent_call(crate::ai_service::AgentCall {
                role: &Role::ASSISTANT,
                caller: &caller,
                binding: &self.binding,
                started,
                request,
                outcome,
            })
            .await;
    }

    fn call_key(call: Option<&oxplow_ai_rig::rig_core::message::CallId>) -> String {
        call.map(ToString::to_string).unwrap_or_default()
    }
}

impl AgentHook for TurnHook {
    async fn on_text_delta(&self, _ctx: &HookContext, event: TextDelta<'_>) -> ObservationAction {
        self.session
            .chunk(ChatUpdate::AgentChunk(event.delta.to_string()));
        ObservationAction::continue_run()
    }

    async fn on_reasoning_delta(
        &self,
        _ctx: &HookContext,
        event: ReasoningDelta<'_>,
    ) -> ObservationAction {
        self.session
            .chunk(ChatUpdate::ThoughtChunk(event.delta.to_string()));
        ObservationAction::continue_run()
    }

    async fn on_dispatch(&self, _ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        if let Some(request) = event.completion_request() {
            // What the record keeps: the conversation and the tools' names
            // (their schemas are the commands').
            let body = json!({
                "assistant": self.session.id,
                "messages": request.chat_history,
                "tools": request.tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>(),
            });
            self.asked
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(event.id, (Instant::now(), body));
            return DispatchAction::proceed();
        }
        let Some(name) = event.tool_name() else {
            return DispatchAction::proceed();
        };
        let args: Value = event
            .tool_args()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or(Value::Null);
        let call = Started::of(name, self.specs.get(name), args, &self.bus);
        let (item, ask) = self.session.call_started(call);
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(Self::call_key(event.call_id), item);
        if let Some(answer) = ask {
            match answer.await {
                Ok(true) => self
                    .session
                    .call_update(item, |c| c.status = ToolStatus::InProgress),
                Ok(false) => {
                    self.session.call_update(item, |c| {
                        c.status = ToolStatus::Failed;
                        c.text.push("Declined".into());
                    });
                    return DispatchAction::skip(DECLINED);
                }
                Err(_) => return DispatchAction::stop("the turn was stopped"),
            }
        }
        DispatchAction::proceed()
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        let asked = self
            .asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&event.id);
        if let Some((started, request)) = asked {
            self.record(&event, started, request).await;
            return OutcomeAction::proceed();
        }
        if event.tool_name().is_none() {
            return OutcomeAction::proceed();
        }
        let item = self
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&Self::call_key(event.call_id));
        let Some(item) = item else {
            return OutcomeAction::proceed();
        };
        let audit = event
            .context
            .and_then(|c| c.result::<AuditRef>().ok().flatten())
            .map(|a| a.0);
        let result = event.tool_result();
        self.session.call_update(item, |c| {
            c.audit_id = audit;
            match result {
                Some(r) if r.is_success() => {
                    c.status = ToolStatus::Completed;
                    let output = r.output();
                    c.raw_output = output
                        .as_json()
                        .cloned()
                        .or_else(|| output.as_text().map(|t| Value::String(t.into())));
                }
                Some(r) => {
                    c.status = ToolStatus::Failed;
                    if let Some(error) = r.error() {
                        c.text.push(error.message().to_string());
                    }
                }
                None => c.status = ToolStatus::Failed,
            }
        });
        OutcomeAction::proceed()
    }
}
