//! An external provider's agent harness: [`ExternalHarness`] is the
//! [`AgentHarness`] registered under the instance's id while it runs
//! (`.context/agent-model.md`, `.context/providers.md` "What a provider may
//! implement"). What a built-in answers in code it declared — its
//! features (a `structured_transcript` is a chat) and its capability's
//! `data` (instruction files, environment markers, settings); each
//! operation is one invoke of the verb of its name:
//!
//! - `launch` — the launch's input as it is → a `Launch`
//!   (`{ spec: { kind: pty|acp, … }, resume_dropped? }`);
//! - `tool_use { body }` → `{ tool }` (`null`: the body names no tool);
//! - `render { answer }` → `{ body }`;
//! - `refresh_text { roots, text }` → `{}`;
//! - `turns { transcript }` → `{ turns }`;
//! - `token_readings { records }` → `{ readings }`, one export's records at
//!   once;
//! - `prompt { body }` → `{ prompt }` (`{ kind: person, text }` or
//!   `{ kind: handback, subagent }`, `null` for none) and `subagent { body }`
//!   → `{ subagent }` (a `SubagentStart` / `SubagentStop`'s), its
//!   `subagents` feature.
//!
//! An optional verb it doesn't declare answers none without a call. It
//! emits nothing. `tool_use` and `render` run on the hook route's path,
//! so each is bounded by [`HOOK_VERB_TIMEOUT`]: a provider that doesn't
//! answer in time (or fails) reads as no call and the empty answer, the
//! route's own fail-open stance.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxplow_domain::agent::harness::{
    AgentHarness, HarnessData, HarnessError, HarnessSetting, Interact, Launch, LaunchInput,
    RuntimeRoots, Transcript,
};
use oxplow_domain::agent::observe::{HookAnswer, OtlpRecord, Prompt, TokenReading, Turn};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent::tool::{Subagent, ToolUse};
use oxplow_domain::capability::AGENT_HARNESS;
use oxplow_domain::InputValidator;
use serde_json::{json, Value};

use super::registry::{CapabilityHost, Instance};

/// How long the hook route waits on a provider's `tool_use` or `render`,
/// inside its own budget (`HOOK_HANDLING_TIMEOUT`, 5 s).
pub const HOOK_VERB_TIMEOUT: Duration = Duration::from_secs(1);

/// The agent harness's side of the host: a started instance is a harness
/// in `Services.harnesses`, under its id.
pub struct HarnessHost(pub HarnessRegistry);

impl HarnessHost {
    pub const CAPABILITY: &'static str = "agent_harness";
}

impl CapabilityHost for HarnessHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.0.has(id)
    }

    fn admit(&self, instance: &Arc<Instance>) -> Result<Value, String> {
        let declared = &instance.declared;
        let capability = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it declares no `agent_harness` capability")?;
        let features = Some(capability.features.clone())
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        let data: HarnessData = match &capability.data {
            Value::Null => HarnessData::default(),
            data => serde_json::from_value(data.clone()).map_err(|e| format!("its data: {e}"))?,
        };
        // Each of the contract's verbs it declares, its input checked
        // against what it declared.
        let mut verbs = BTreeMap::new();
        for verb in AGENT_HARNESS.verbs {
            if let Some(c) = declared.commands.iter().find(|c| c.name == verb.name) {
                let input = InputValidator::compile(&c.input_schema)
                    .map_err(|e| format!("verb `{}`: {e}", verb.name))?;
                verbs.insert(verb.name, input);
            }
        }
        let structured = features
            .get("structured_transcript")
            .and_then(Value::as_bool)
            == Some(true);
        self.0.register(Arc::new(ExternalHarness {
            instance: instance.clone(),
            title: declared.provider.name.clone(),
            interact: Interact {
                transcript: if structured {
                    Transcript::Structured
                } else {
                    Transcript::Terminal
                },
            },
            data,
            verbs,
        }));
        Ok(features)
    }

    fn retire(&self, id: &str) {
        self.0.unregister(id);
    }
}

pub struct ExternalHarness {
    instance: Arc<Instance>,
    title: String,
    interact: Interact,
    data: HarnessData,
    /// The verbs it declares, their input schemas compiled.
    verbs: BTreeMap<&'static str, InputValidator>,
}

impl ExternalHarness {
    /// `verb`'s answer to `input`; `Ok(None)` when it doesn't declare the
    /// verb (no call is made).
    async fn call(
        &self,
        verb: &str,
        input: Value,
        key: Option<String>,
    ) -> Result<Option<Value>, String> {
        let Some(schema) = self.verbs.get(verb) else {
            return Ok(None);
        };
        schema
            .check(&input)
            .map_err(|e| format!("its `{verb}` schema refuses the input: {e}"))?;
        let out = self
            .instance
            .invoke(verb, input, key)
            .await
            .map_err(|e| e.to_string())?;
        if !out.events.is_empty() {
            return Err(format!(
                "`{verb}` returned events, and a harness emits none"
            ));
        }
        Ok(Some(out.result))
    }

    /// [`Self::call`] on the hook path: bounded, and a failure logged and
    /// read as no answer.
    async fn hook_call(&self, verb: &str, input: Value) -> Option<Value> {
        match tokio::time::timeout(HOOK_VERB_TIMEOUT, self.call(verb, input, None)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => {
                tracing::warn!(harness = %self.instance.name, verb, %error, "a harness provider's hook verb failed");
                None
            }
            Err(_) => {
                tracing::warn!(harness = %self.instance.name, verb, "a harness provider's hook verb timed out");
                None
            }
        }
    }

    /// A failure off the hook path, logged; the answer is none.
    fn logged<T: Default>(&self, verb: &str, answer: Result<Option<T>, String>) -> T {
        match answer {
            Ok(answer) => answer.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(harness = %self.instance.name, verb, %error, "a harness provider's verb failed");
                T::default()
            }
        }
    }

    fn failed(&self, verb: &str, message: impl std::fmt::Display) -> String {
        format!("harness `{}` `{verb}`: {message}", self.instance.name)
    }
}

/// `answer[field]` as a `T`.
fn field<T: serde::de::DeserializeOwned>(answer: Value, field: &str) -> Result<T, String> {
    let mut answer = answer;
    serde_json::from_value(
        answer
            .get_mut(field)
            .map(Value::take)
            .unwrap_or(Value::Null),
    )
    .map_err(|e| format!("its answer's `{field}`: {e}"))
}

#[async_trait]
impl AgentHarness for ExternalHarness {
    fn id(&self) -> &str {
        &self.instance.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn interact(&self) -> Interact {
        self.interact
    }

    async fn launch(&self, input: &LaunchInput) -> Result<Launch, HarnessError> {
        let params =
            serde_json::to_value(input).map_err(|e| HarnessError::Config(e.to_string()))?;
        // Each launch is its own call: a relaunch reads what's true now.
        let answer = self
            .call("launch", params, None)
            .await
            .map_err(|e| HarnessError::Config(self.failed("launch", e)))?
            .ok_or_else(|| {
                HarnessError::Config(self.failed("launch", "it declares no `launch`"))
            })?;
        serde_json::from_value(answer)
            .map_err(|e| HarnessError::Config(self.failed("launch", format!("its answer: {e}"))))
    }

    async fn tool_use(&self, body: &Value) -> Option<ToolUse> {
        let answer = self.hook_call("tool_use", json!({ "body": body })).await?;
        match field::<Option<ToolUse>>(answer, "tool") {
            Ok(tool) => tool,
            Err(error) => {
                tracing::warn!(harness = %self.instance.name, %error, "a harness provider's tool_use answer doesn't read");
                None
            }
        }
    }

    async fn render(&self, answer: &HookAnswer) -> Value {
        let Some(out) = self.hook_call("render", json!({ "answer": answer })).await else {
            return json!({});
        };
        match field::<Value>(out, "body") {
            Ok(body) if !body.is_null() => body,
            _ => json!({}),
        }
    }

    fn instruction_files(&self) -> Vec<String> {
        self.data.instruction_files.clone()
    }

    fn env_markers(&self) -> Vec<String> {
        self.data.env_markers.clone()
    }

    fn settings(&self) -> Vec<HarnessSetting> {
        self.data.settings.clone()
    }

    async fn refresh_text(
        &self,
        roots: &RuntimeRoots,
        text: &AgentText,
    ) -> Result<(), HarnessError> {
        self.call(
            "refresh_text",
            json!({ "roots": roots, "text": text }),
            None,
        )
        .await
        .map(|_| ())
        .map_err(|e| HarnessError::Runtime(self.failed("refresh_text", e)))
    }

    async fn turns(&self, transcript: &str) -> Vec<Turn> {
        let answer = self
            .call("turns", json!({ "transcript": transcript }), None)
            .await
            .and_then(|a| a.map(|a| field(a, "turns")).transpose());
        self.logged("turns", answer)
    }

    /// A harness without `subagents` reads a prompt as a person's (the
    /// default); one with it says, on the hook path.
    async fn prompt(&self, body: &Value) -> Option<Prompt> {
        if !self.verbs.contains_key("prompt") {
            return body
                .get("prompt")
                .and_then(|p| p.as_str())
                .map(|text| Prompt::Person { text: text.into() });
        }
        let answer = self.hook_call("prompt", json!({ "body": body })).await?;
        field::<Option<Prompt>>(answer, "prompt").ok().flatten()
    }

    async fn subagent(&self, body: &Value) -> Option<Subagent> {
        let answer = self.hook_call("subagent", json!({ "body": body })).await?;
        field::<Option<Subagent>>(answer, "subagent").ok().flatten()
    }

    async fn token_readings(&self, records: &[OtlpRecord]) -> Vec<TokenReading> {
        let answer = self
            .call("token_readings", json!({ "records": records }), None)
            .await
            .and_then(|a| a.map(|a| field(a, "readings")).transpose());
        self.logged("token_readings", answer)
    }
}
