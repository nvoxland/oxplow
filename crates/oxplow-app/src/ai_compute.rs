//! Recorded AI computations (P5.E1, `.context/ai-providers.md` "Recorded
//! computations"): `classify`, `score`, `decide`, `summarize` and
//! `extract`, each kept in `ai_result` by the hash of its input, the
//! provider, the model and the request the model is sent — the whole
//! prompt, so a changed prompt is a new result without a version to bump.
//! Asking again for the same thing reads the recorded result — no call,
//! no `ai_call` row — so a computation is paid for once and reads stay
//! deterministic. Tokens only; there is no cost.
//!
//! Each runs on the role its caller names — a built-in or an extension's
//! AI profile (the collector builtins default to `decide`, `summarize`
//! and `main`):
//! - `classify(caller, role, text, labels)` and `score(…, levels)` ask a
//!   typed question, and `decide(caller, role, state, questions)` asks
//!   any set of them;
//! - `summarize(caller, role, text, focus)` summarizes;
//! - `extract(caller, role, instructions, text, schema)` asks for JSON
//!   matching `schema`, and refuses a reply that doesn't.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_ai::client::{Answer, Question};
use oxplow_ai::config::Role;
use oxplow_db::{NewAiResult, SqliteAiResultStore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai_service::{
    AiService, AiServiceError, CallSite, ModelRequest, TypedRequestV1, TypedResponseV1,
};

/// What `classify` asks of the text.
const CLASSIFY_INSTRUCTIONS: &str = "Which label fits the text best?";
/// What `score` asks of the text.
const SCORE_INSTRUCTIONS: &str = "Where does the text sit on this scale?";

#[derive(Debug, thiserror::Error)]
pub enum AiComputeError {
    #[error(transparent)]
    Ai(#[from] AiServiceError),
    /// The model answered, but not in the shape asked for. Nothing is
    /// recorded.
    #[error("the model's answer isn't usable: {0}")]
    BadOutput(String),
    #[error("recording the result: {0}")]
    Storage(String),
}

/// A computation's result, and whether it was read back rather than
/// computed now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recorded<T> {
    pub value: T,
    /// Read from `ai_result`: no call was made.
    pub cached: bool,
    /// The call that computed it (`v_ai_call.id`).
    pub ai_call_id: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// `classify`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub label: String,
    pub probabilities: BTreeMap<String, f64>,
}

/// `score`'s answer: the level, its probability-weighted index (0 = the
/// first level) and each level's probability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scored {
    pub level: String,
    pub score: f64,
    pub probabilities: BTreeMap<String, f64>,
}

/// The key a computation is recorded by: sha256 of `{ op, args }` as
/// canonical JSON (object keys sorted, no whitespace).
pub fn input_hash(op: &str, args: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut text = String::new();
    canonical(&json!({ "op": op, "args": args }), &mut text);
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

pub struct AiCompute {
    ai: Arc<AiService>,
    results: SqliteAiResultStore,
}

/// Who asks for a computation: what (`oxplow:oxplow/collector?collector=<owner>/<id>`,
/// `oxplow:oxplow/thread?id=<id>`, …) and, for an extension's, its run.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    pub name: &'a str,
    pub run: Option<&'a str>,
}

impl<'a> From<&'a str> for Caller<'a> {
    fn from(name: &'a str) -> Self {
        Caller { name, run: None }
    }
}

/// One computation, before it runs: what it is (`name`, `args`), the
/// request it sends, on which role, for whom.
struct Op<'a> {
    name: &'static str,
    role: &'a Role,
    caller: Caller<'a>,
    args: Value,
    request: ModelRequest<'a>,
}

impl AiCompute {
    pub fn new(ai: Arc<AiService>, results: SqliteAiResultStore) -> Self {
        Self { ai, results }
    }

    /// The model `role` is bound to now: what a job's request names.
    pub(crate) fn model_for(&self, role: &Role) -> Result<String, AiComputeError> {
        Ok(self.ai.binding_for(role)?.model)
    }

    /// One model round of a job: its request streamed on `role` (a
    /// recorded computation like any other, so a retried job's identical
    /// round reads the recorded answer), under the job run's authority.
    pub(crate) async fn stream_job_round(
        &self,
        run: &oxplow_domain::run::RunContext,
        role: &Role,
        request: &TypedRequestV1,
        on_delta: oxplow_ai::client::OnDelta<'_>,
    ) -> Result<Recorded<TypedResponseV1>, AiComputeError> {
        let refused = || AiComputeError::BadOutput("job AI authority was refused".into());
        if !matches!(
            run.decide_service("ai.compute"),
            oxplow_domain::grant::Decision::Allowed
        ) {
            return Err(refused());
        }
        let name = run.actor().source();
        // Not under the job run's own pin: a job holds no generation, and
        // the model's binding is resolved under one of its own.
        run.check_authority().map_err(|_| refused())?;
        run.used_ai();
        self.stream_typed(
            Caller {
                name: &name,
                run: Some(run.run().as_str()),
            },
            role,
            request,
            run.cancel(),
            on_delta,
        )
        .await
    }

    /// Read `op`'s recorded result, or run `compute` (given the input
    /// hash) and record what it returns.
    async fn recorded<T, F, Fut>(
        &self,
        op: Op<'_>,
        compute: F,
    ) -> Result<Recorded<T>, AiComputeError>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce(String, crate::ai_service::Bound) -> Fut,
        Fut: std::future::Future<Output = Result<(T, i64, i64, Option<i64>), AiComputeError>>,
    {
        // Bound once: the result is keyed by, and computed on, the same
        // provider and model, though no generation is held across the call.
        let bound = self.ai.bind(op.role).await?;
        let binding = bound.binding().clone();
        let hash = input_hash(op.name, &op.args);
        let request_hash = op.request.hash();
        let storage = |e: oxplow_domain::DomainError| AiComputeError::Storage(e.to_string());
        if let Some(hit) = self
            .results
            .get(&hash, &binding.provider, &binding.model, &request_hash)
            .await
            .map_err(storage)?
        {
            return Ok(Recorded {
                value: serde_json::from_value(hit.output)
                    .map_err(|e| AiComputeError::Storage(format!("recorded output: {e}")))?,
                cached: true,
                ai_call_id: hit.ai_call_id,
                input_tokens: hit.input_tokens,
                output_tokens: hit.output_tokens,
            });
        }
        let (value, input_tokens, output_tokens, ai_call_id) = compute(hash.clone(), bound).await?;
        // A concurrent computation of the same thing may have recorded
        // first: what's recorded is the answer, so that one is returned.
        let kept = self
            .results
            .insert(NewAiResult {
                input_hash: hash,
                provider: binding.provider,
                model: binding.model,
                request_hash,
                op: op.name.into(),
                role: op.role.to_string(),
                caller: op.caller.name.into(),
                output: serde_json::to_value(&value).expect("result serializes"),
                input_tokens,
                output_tokens,
                ai_call_id,
            })
            .await
            .map_err(storage)?;
        Ok(Recorded {
            value: serde_json::from_value(kept.output)
                .map_err(|e| AiComputeError::Storage(format!("recorded output: {e}")))?,
            cached: kept.ai_call_id != ai_call_id,
            ai_call_id: kept.ai_call_id,
            input_tokens: kept.input_tokens,
            output_tokens: kept.output_tokens,
        })
    }

    /// An ordered, typed request, cached with its full request and role binding.
    pub async fn complete_typed(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        request: &TypedRequestV1,
    ) -> Result<Recorded<TypedResponseV1>, AiComputeError> {
        request
            .validate_version()
            .map_err(oxplow_ai::client::AiError::from)
            .map_err(AiServiceError::from)?;
        let caller = caller.into();
        let op = Op {
            name: "completeTyped",
            role,
            caller,
            args: serde_json::to_value(request).expect("request serializes"),
            request: ModelRequest::Typed(request),
        };
        self.recorded(op, |hash, bound| async move {
            let (response, call) = self
                .ai
                .complete_typed_on(&bound, site(caller, &hash), request)
                .await?;
            let (input, output) = crate::ai_service::observed_counts(&response);
            Ok((response, input, output, call))
        })
        .await
    }

    /// [`Self::complete_typed`], streamed: the reply goes to `on_delta` as
    /// it arrives, until `cancel` is asked. A recorded answer is handed
    /// over whole, at once, with no call. A cancelled or failed stream is
    /// never recorded as the answer (its call row keeps what came).
    pub async fn stream_typed(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        request: &TypedRequestV1,
        cancel: &oxplow_ai::client::Cancel,
        on_delta: oxplow_ai::client::OnDelta<'_>,
    ) -> Result<Recorded<TypedResponseV1>, AiComputeError> {
        request
            .validate_version()
            .map_err(oxplow_ai::client::AiError::from)
            .map_err(AiServiceError::from)?;
        let caller = caller.into();
        let op = Op {
            name: "completeTyped",
            role,
            caller,
            args: serde_json::to_value(request).expect("request serializes"),
            request: ModelRequest::Typed(request),
        };
        let mut on_delta = on_delta;
        let mut streamed = false;
        let recorded = {
            let (sink, streamed) = (&mut on_delta, &mut streamed);
            self.recorded(op, |hash, bound| async move {
                *streamed = true;
                let (response, call) = self
                    .ai
                    .stream_typed_on(&bound, site(caller, &hash), request, cancel, &mut **sink)
                    .await?;
                let (input, output) = crate::ai_service::observed_counts(&response);
                Ok((response, input, output, call))
            })
            .await?
        };
        if !streamed {
            for delta in oxplow_ai::parts::StreamDeltaV1::of(&recorded.value.parts) {
                on_delta(delta);
            }
        }
        Ok(recorded)
    }

    /// Which of `labels` fits `text` (asked as a typed choice; the
    /// `decide` role unless the caller names another).
    pub async fn classify(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        text: &str,
        labels: &[String],
    ) -> Result<Recorded<Classification>, AiComputeError> {
        let caller = caller.into();
        let questions = BTreeMap::from([(
            "label".to_string(),
            Question::Choice {
                instructions: CLASSIFY_INSTRUCTIONS.into(),
                options: labels.to_vec(),
            },
        )]);
        let op = Op {
            name: "classify",
            role,
            caller,
            args: json!({ "text": text, "labels": labels }),
            request: ModelRequest::Decide {
                state: text,
                questions: &questions,
            },
        };
        let questions = &questions;
        self.recorded(op, |hash, bound| async move {
            let (decision, call) = self
                .ai
                .decide_on(&bound, site(caller, &hash), text, questions)
                .await?;
            match decision.answers.get("label") {
                Some(Answer::Choice {
                    choice,
                    probabilities,
                }) if labels.contains(choice) => Ok((
                    Classification {
                        label: choice.clone(),
                        probabilities: probabilities.clone(),
                    },
                    decision.input_tokens,
                    decision.output_tokens,
                    call,
                )),
                other => Err(AiComputeError::BadOutput(format!(
                    "expected one of {labels:?}, got {other:?}"
                ))),
            }
        })
        .await
    }

    /// Where `text` sits on `levels`, lowest first (a typed score).
    pub async fn score(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        text: &str,
        levels: &[String],
    ) -> Result<Recorded<Scored>, AiComputeError> {
        let caller = caller.into();
        let questions = BTreeMap::from([(
            "score".to_string(),
            Question::Score {
                instructions: SCORE_INSTRUCTIONS.into(),
                levels: levels.to_vec(),
            },
        )]);
        let op = Op {
            name: "score",
            role,
            caller,
            args: json!({ "text": text, "levels": levels }),
            request: ModelRequest::Decide {
                state: text,
                questions: &questions,
            },
        };
        let questions = &questions;
        self.recorded(op, |hash, bound| async move {
            let (decision, call) = self
                .ai
                .decide_on(&bound, site(caller, &hash), text, questions)
                .await?;
            match decision.answers.get("score") {
                Some(Answer::Score {
                    score,
                    probabilities,
                }) if !levels.is_empty() => {
                    let index = (score.round().max(0.0) as usize).min(levels.len() - 1);
                    Ok((
                        Scored {
                            level: levels[index].clone(),
                            score: *score,
                            probabilities: probabilities.clone(),
                        },
                        decision.input_tokens,
                        decision.output_tokens,
                        call,
                    ))
                }
                other => Err(AiComputeError::BadOutput(format!(
                    "expected a score on {levels:?}, got {other:?}"
                ))),
            }
        })
        .await
    }

    /// Answers to typed `questions` about `state`, by question name. A reply missing an answer is refused.
    pub async fn decide(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Recorded<BTreeMap<String, Answer>>, AiComputeError> {
        let caller = caller.into();
        let op = Op {
            name: "decide",
            role,
            caller,
            args: json!({ "state": state, "questions": questions }),
            request: ModelRequest::Decide { state, questions },
        };
        self.recorded(op, |hash, bound| async move {
            let (decision, call) = self
                .ai
                .decide_on(&bound, site(caller, &hash), state, questions)
                .await?;
            if let Some(missing) = questions
                .keys()
                .find(|q| !decision.answers.contains_key(*q))
            {
                return Err(AiComputeError::BadOutput(format!(
                    "no answer to question `{missing}`"
                )));
            }
            Ok((
                decision.answers,
                decision.input_tokens,
                decision.output_tokens,
                call,
            ))
        })
        .await
    }

    /// A summary of `text`, focused on `focus`.
    pub async fn summarize(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        text: &str,
        focus: Option<&str>,
    ) -> Result<Recorded<String>, AiComputeError> {
        let caller = caller.into();
        let system = crate::ai_service::summarize_system(focus);
        let op = Op {
            name: "summarize",
            role,
            caller,
            args: json!({ "text": text, "focus": focus }),
            request: ModelRequest::Complete {
                system: Some(&system),
                prompt: text,
                json: false,
            },
        };
        let system = &system;
        self.recorded(op, |hash, bound| async move {
            let (c, call) = self
                .ai
                .complete_on(&bound, site(caller, &hash), Some(system), text, false)
                .await?;
            Ok((
                c.text.trim().to_string(),
                c.input_tokens,
                c.output_tokens,
                call,
            ))
        })
        .await
    }

    /// JSON matching `schema` from `text`, as `instructions` ask (in JSON
    /// mode). A reply that isn't JSON, or doesn't
    /// match, is refused and not recorded.
    pub async fn extract(
        &self,
        caller: impl Into<Caller<'_>>,
        role: &Role,
        instructions: &str,
        text: &str,
        schema: &Value,
    ) -> Result<Recorded<Value>, AiComputeError> {
        let caller = caller.into();
        let system =
            format!("{instructions}\n\nReply with JSON only, matching this JSON Schema:\n{schema}");
        let op = Op {
            name: "extract",
            role,
            caller,
            args: json!({ "instructions": instructions, "text": text, "schema": schema }),
            request: ModelRequest::Complete {
                system: Some(&system),
                prompt: text,
                json: true,
            },
        };
        let system = &system;
        self.recorded(op, |hash, bound| async move {
            let validator = oxplow_domain::InputValidator::compile(schema).map_err(|e| {
                AiComputeError::BadOutput(format!("the schema doesn't compile: {e}"))
            })?;
            let (c, call) = self
                .ai
                .complete_on(&bound, site(caller, &hash), Some(system), text, true)
                .await?;
            let value = parse_json_reply(&c.text)?;
            if let Err(oxplow_domain::CommandError::Invalid { field, message }) =
                validator.check(&value)
            {
                return Err(AiComputeError::BadOutput(format!(
                    "the reply doesn't match the schema at `{}`: {message}",
                    field.unwrap_or_else(|| "/".into())
                )));
            }
            Ok((value, c.input_tokens, c.output_tokens, call))
        })
        .await
    }
}

/// [`AiCompute`] as a collector's `ai_*` oracle (`oxplow_script`),
/// recording as `caller` (`oxplow:oxplow/collector?collector=<owner>/<id>`). The script runs on a
/// worker thread outside the runtime; each call blocks it on the runtime
/// the oracle was made on.
pub struct CollectorOracle {
    compute: Arc<AiCompute>,
    caller: String,
    /// The collector's run: each call is counted on it and names it.
    run: oxplow_domain::run::RunContext,
    runtime: tokio::runtime::Handle,
}

impl CollectorOracle {
    /// Made inside the runtime its calls run on, for `run`.
    pub fn new(
        compute: Arc<AiCompute>,
        caller: String,
        run: oxplow_domain::run::RunContext,
    ) -> Self {
        Self {
            compute,
            caller,
            run,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    fn caller(&self) -> Caller<'_> {
        Caller {
            name: &self.caller,
            run: Some(self.run.run().as_str()),
        }
    }

    /// `role` as the collector's own extension means it: a built-in, or
    /// the bare name of one of its AI profiles, named in its namespace.
    fn role(&self, role: &str) -> Result<Role, String> {
        match self.run.namespace() {
            Some(namespace) => Role::parse_in(namespace, role),
            None => Role::parse(role),
        }
    }

    fn metadata<T: Serialize>(
        &self,
        future: impl std::future::Future<Output = Result<T, AiServiceError>>,
    ) -> Result<Value, String> {
        if let oxplow_domain::grant::Decision::Denied(reason) =
            self.run.decide_service("ai.discover")
        {
            return Err(reason);
        }
        let result = self.runtime.block_on(async {
            let future = self.compute.ai.read_run(&self.run, future);
            tokio::pin!(future);
            loop {
                tokio::select! {
                    result = &mut future => return result.map_err(|e| e.to_string())?.map_err(|e| e.to_string()),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        self.run.check_authority()?;
                    }
                }
            }
        });
        self.run.check_authority()?;
        result.and_then(|metadata| serde_json::to_value(metadata).map_err(|e| e.to_string()))
    }

    fn run<T: Serialize>(
        &self,
        f: impl std::future::Future<Output = Result<Recorded<T>, AiComputeError>>,
    ) -> Result<Value, String> {
        if let oxplow_domain::grant::Decision::Denied(reason) =
            self.run.decide_service("ai.compute")
        {
            return Err(reason);
        }
        self.run.used_ai();
        let result = self.runtime.block_on(async {
            let f = self.compute.ai.read_run(&self.run, f);
            tokio::pin!(f);
            loop {
                tokio::select! {
                    result = &mut f => return result.map_err(|e| e.to_string())?.map_err(|e| e.to_string()),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        self.run.check_authority()?;
                    }
                }
            }
        });
        self.run.check_authority()?;
        result.map(|r| serde_json::to_value(r.value).expect("result serializes"))
    }
}

impl oxplow_script::AiOracle for CollectorOracle {
    fn capabilities(&self, role: &str) -> Result<Value, String> {
        let role = self.role(role)?;
        self.metadata(self.compute.ai.capabilities(&role))
    }

    fn models(&self, role: &str) -> Result<Value, String> {
        let role = self.role(role)?;
        self.metadata(self.compute.ai.models(&role))
    }

    fn complete_typed(&self, role: &str, mut request: Value) -> Result<Value, String> {
        if let oxplow_domain::grant::Decision::Denied(reason) =
            self.run.decide_service("ai.compute")
        {
            return Err(reason);
        }
        let role = self.role(role)?;
        let fields = request
            .as_object_mut()
            .ok_or("typed request must be an object")?;
        if fields.contains_key("model") {
            return Err("a collector selects an AI role, not a model".into());
        }
        self.run(async {
            let binding = self.compute.ai.binding_for(&role)?;
            // Resolve the role inside the same captured AI configuration as
            // cache lookup, provider invocation and result attribution.
            request
                .as_object_mut()
                .expect("validated typed request")
                .insert("model".into(), Value::String(binding.model));
            let request = serde_json::from_value(request)
                .map_err(|_| AiComputeError::BadOutput("invalid typed request".into()))?;
            self.compute
                .complete_typed(self.caller(), &role, &request)
                .await
        })
    }

    fn classify(&self, role: &str, text: &str, labels: &[String]) -> Result<Value, String> {
        let role = self.role(role)?;
        self.run(self.compute.classify(self.caller(), &role, text, labels))
    }

    fn score(&self, role: &str, text: &str, levels: &[String]) -> Result<Value, String> {
        let role = self.role(role)?;
        self.run(self.compute.score(self.caller(), &role, text, levels))
    }

    fn summarize(&self, role: &str, text: &str, focus: Option<&str>) -> Result<String, String> {
        let role = self.role(role)?;
        self.run(self.compute.summarize(self.caller(), &role, text, focus))
            .map(|v| v.as_str().unwrap_or_default().to_string())
    }

    fn extract(
        &self,
        role: &str,
        instructions: &str,
        text: &str,
        schema: &Value,
    ) -> Result<Value, String> {
        let role = self.role(role)?;
        self.run(
            self.compute
                .extract(self.caller(), &role, instructions, text, schema),
        )
    }
}

fn site<'a>(caller: Caller<'a>, hash: &'a str) -> CallSite<'a> {
    CallSite {
        caller: caller.name,
        input_hash: Some(hash),
        run: caller.run,
    }
}

/// A JSON reply, tolerating a Markdown code fence around it.
fn parse_json_reply(text: &str) -> Result<Value, AiComputeError> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    serde_json::from_str(t)
        .map_err(|e| AiComputeError::BadOutput(format!("the reply isn't JSON ({e})")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_service::{ProviderConfig, RoleBinding};

    async fn with_model(svc: &crate::Services, role: Role, reply: Value) {
        with_provider(svc, role, "m", reply).await
    }

    /// Bind `role` to model `x` served by provider `provider`.
    async fn with_provider(svc: &crate::Services, role: Role, provider: &str, reply: Value) {
        let (base, _) = oxplow_ai_fake::mock("/chat/completions", 200, reply).await;
        svc.ai
            .save_provider(
                ProviderConfig {
                    id: provider.into(),
                    kind: "openai_compatible".into(),
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        svc.ai
            .set_role(&role, Some(RoleBinding::new(provider, "x")))
            .unwrap();
    }

    fn chat(content: &str) -> Value {
        json!({ "choices": [{ "index": 0, "finish_reason": "stop", "message": { "role": "assistant", "content": content } }], "usage": { "prompt_tokens": 7, "completion_tokens": 3 } })
    }

    async fn calls(svc: &crate::Services) -> Vec<(String, Option<String>)> {
        svc.db
            .read(|c| {
                let mut stmt = c
                    .prepare("SELECT caller, input_hash FROM ai_call ORDER BY id")
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .and_then(|r| r.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// P5.E1's red: the same computation twice is one call; the second
    /// reads the recorded result.
    #[tokio::test]
    async fn the_same_computation_twice_is_one_call_and_a_cached_result() {
        let fx = crate::test_fixtures::services_with_effort().await;
        with_model(&fx.svc, Role::SUMMARIZE, chat("  A short summary.  ")).await;
        let first = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "a long text", None)
            .await
            .unwrap();
        assert_eq!(first.value, "A short summary.");
        assert!(!first.cached);
        assert_eq!((first.input_tokens, first.output_tokens), (7, 3));
        let second = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "a long text", None)
            .await
            .unwrap();
        assert!(second.cached);
        assert_eq!(second.value, first.value);
        assert_eq!(second.ai_call_id, first.ai_call_id);
        let recorded = calls(&fx.svc).await;
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(
            recorded[0].1.as_deref(),
            Some(
                input_hash(
                    "summarize",
                    &json!({ "text": "a long text", "focus": null })
                )
                .as_str()
            )
        );

        // The result is keyed by the exact request its call kept — the
        // system prompt included, so editing it computes afresh.
        let system = crate::ai_service::summarize_system(None);
        let request = ModelRequest::Complete {
            system: Some(&system),
            prompt: "a long text",
            json: false,
        };
        let keys = fx
            .svc
            .sql
            .query_sql(
                "SELECT r.request_hash, c.request_hash FROM v_ai_result r \
                 JOIN v_ai_call c ON c.id = r.ai_call_id",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&keys.rows).unwrap(),
            json!([[request.hash(), request.hash()]])
        );

        // Another input, or focus, is another computation.
        let other = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "a long text", Some("risks"))
            .await
            .unwrap();
        assert!(!other.cached);
        assert_eq!(calls(&fx.svc).await.len(), 2);
    }

    /// Typed questions about a text are a recorded computation on the
    /// `decide` role: asked twice, one call, recorded as its caller; a
    /// reply that leaves a question unanswered isn't recorded.
    #[tokio::test]
    async fn decide_is_a_recorded_computation() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let reply = json!({ "answers": { "risky": { "type": "noul", "probability": 0.8 } } });
        with_model(&fx.svc, Role::DECIDE, chat(&reply.to_string())).await;
        let questions = BTreeMap::from([(
            "risky".to_string(),
            Question::Noul {
                instructions: "Is it risky?".into(),
            },
        )]);
        for cached in [false, true] {
            let out = fx
                .svc
                .ai_compute
                .decide(
                    "oxplow:oxplow/thread?id=thr1",
                    &Role::DECIDE,
                    "a diff",
                    &questions,
                )
                .await
                .unwrap();
            assert_eq!(out.cached, cached);
            assert_eq!(out.value["risky"], Answer::Noul { probability: 0.8 });
        }
        let recorded = calls(&fx.svc).await;
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].0, "oxplow:oxplow/thread?id=thr1");

        let more = BTreeMap::from([
            ("risky".to_string(), questions["risky"].clone()),
            (
                "big".to_string(),
                Question::Noul {
                    instructions: "Is it big?".into(),
                },
            ),
        ]);
        let err = fx
            .svc
            .ai_compute
            .decide(
                "oxplow:oxplow/thread?id=thr1",
                &Role::DECIDE,
                "a diff",
                &more,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("`big`"), "{err}");
    }

    /// A result is the provider's as well as the model's: rebinding a role
    /// to another provider serving the same model name computes afresh.
    #[tokio::test]
    async fn another_provider_of_the_same_model_computes_afresh() {
        let fx = crate::test_fixtures::services_with_effort().await;
        with_provider(&fx.svc, Role::SUMMARIZE, "local", chat("local says")).await;
        let local = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "t", None)
            .await
            .unwrap();
        assert_eq!(local.value, "local says");
        with_provider(&fx.svc, Role::SUMMARIZE, "hosted", chat("hosted says")).await;
        let hosted = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "t", None)
            .await
            .unwrap();
        assert!(!hosted.cached);
        assert_eq!(hosted.value, "hosted says");
        fx.svc
            .ai
            .set_role(&Role::SUMMARIZE, Some(RoleBinding::new("local", "x")))
            .unwrap();
        let again = fx
            .svc
            .ai_compute
            .summarize("test", &Role::SUMMARIZE, "t", None)
            .await
            .unwrap();
        assert!(again.cached);
        assert_eq!(again.value, "local says");
        assert_eq!(calls(&fx.svc).await.len(), 2);
    }

    #[tokio::test]
    async fn extract_refuses_a_reply_that_breaks_the_schema_and_records_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        with_model(
            &fx.svc,
            Role::MAIN,
            chat("```json\n{\"n\": \"three\"}\n```"),
        )
        .await;
        let schema = json!({ "type": "object", "required": ["n"], "properties": { "n": { "type": "integer" } } });
        let refused = fx
            .svc
            .ai_compute
            .extract("test", &Role::MAIN, "Count them.", "one two three", &schema)
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("`/n`"), "{refused}");
        let recorded = fx
            .svc
            .db
            .read(|c| {
                c.query_row("SELECT count(*) FROM ai_result", [], |r| r.get::<_, i64>(0))
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(recorded, 0);
    }

    #[test]
    fn the_input_hash_ignores_key_order() {
        assert_eq!(
            input_hash("x", &json!({ "a": 1, "b": [ { "d": 2, "c": 3 } ] })),
            input_hash("x", &json!({ "b": [ { "c": 3, "d": 2 } ], "a": 1 }))
        );
        assert_ne!(
            input_hash("x", &json!({ "a": 1 })),
            input_hash("y", &json!({ "a": 1 }))
        );
    }
    fn typed_request(text: &str) -> TypedRequestV1 {
        use oxplow_ai::parts::{MessageRoleV1, MessageV1, PartV1};
        TypedRequestV1 {
            schema_version: 1,
            model: "x".into(),
            tools: Vec::new(),
            messages: vec![MessageV1 {
                role: MessageRoleV1::User,
                parts: vec![PartV1::Text { text: text.into() }],
            }],
        }
    }

    /// A typed computation is recorded with its full request and response
    /// under the role's binding: asked twice, one call, and the cached
    /// answer is the same ordered parts with the same usage provenance.
    #[tokio::test]
    async fn a_typed_completion_is_recorded_and_cached_with_provenance() {
        use oxplow_ai::parts::{FinishReasonV1, PartV1, UsageSourceV1};
        let fx = crate::test_fixtures::services_with_effort().await;
        let reply = json!({ "choices": [{ "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": "checking",
            "tool_calls": [{ "id": "c1", "type": "function",
                             "function": { "name": "grep", "arguments": "{\"q\":\"x\"}" } }] } }],
            "usage": { "prompt_tokens": 7, "completion_tokens": 3 } });
        with_model(&fx.svc, Role::SUMMARIZE, reply).await;
        let request = typed_request("find x");
        let first = fx
            .svc
            .ai_compute
            .complete_typed("test", &Role::SUMMARIZE, &request)
            .await
            .unwrap();
        assert!(!first.cached);
        assert_eq!((first.input_tokens, first.output_tokens), (7, 3));
        assert!(matches!(first.value.parts.as_slice(),
            [PartV1::Text { text }, PartV1::ToolCall { name, arguments, .. }]
            if text == "checking" && name == "grep" && arguments["q"] == "x"));
        assert_eq!(first.value.finish_reason, FinishReasonV1::ToolCalls);
        assert_eq!(first.value.usage.source, UsageSourceV1::Observed);

        let second = fx
            .svc
            .ai_compute
            .complete_typed("test", &Role::SUMMARIZE, &request)
            .await
            .unwrap();
        assert!(second.cached);
        assert_eq!(second.value, first.value);
        assert_eq!(second.ai_call_id, first.ai_call_id);
        assert_eq!(calls(&fx.svc).await.len(), 1);

        // The result and its call are keyed by the same typed request.
        let keys = fx
            .svc
            .sql
            .query_sql(
                "SELECT r.request_hash, c.request_hash FROM v_ai_result r \
                 JOIN v_ai_call c ON c.id = r.ai_call_id",
                vec![],
                None,
            )
            .await
            .unwrap();
        let want = ModelRequest::Typed(&request).hash();
        assert_eq!(
            serde_json::to_value(&keys.rows).unwrap(),
            json!([[want, want]])
        );

        // Different parts are another computation.
        let other = fx
            .svc
            .ai_compute
            .complete_typed("test", &Role::SUMMARIZE, &typed_request("find y"))
            .await
            .unwrap();
        assert!(!other.cached);
    }

    /// Images are refused before a call or a recorded result exists, and an
    /// unknown schema version never reaches the provider.
    #[tokio::test]
    async fn a_typed_completion_refuses_images_and_unknown_versions() {
        use oxplow_ai::parts::PartV1;
        let fx = crate::test_fixtures::services_with_effort().await;
        with_model(&fx.svc, Role::SUMMARIZE, chat("never sent")).await;
        let mut with_image = typed_request("see");
        with_image.messages[0].parts.push(PartV1::Image {
            asset_ref: "asset:1".into(),
            media_type: "image/png".into(),
        });
        let err = fx
            .svc
            .ai_compute
            .complete_typed("test", &Role::SUMMARIZE, &with_image)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("imageAssetResolution"), "{err}");
        let stale = TypedRequestV1 {
            schema_version: 2,
            ..typed_request("hi")
        };
        let err = fx
            .svc
            .ai_compute
            .complete_typed("test", &Role::SUMMARIZE, &stale)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("schemaVersion 2"), "{err}");
        assert!(calls(&fx.svc).await.is_empty());
    }
}
