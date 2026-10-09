//! Recorded AI computations (P5.E1, `.context/ai-providers.md` "Recorded
//! computations"): `classify`, `score`, `decide`, `summarize` and
//! `extract`, each kept in `ai_result` by the hash of its input, the
//! provider, the model and the request the model is sent — the whole
//! prompt, so a changed prompt is a new result without a version to bump.
//! Asking again for the same thing reads the recorded result — no call,
//! no `ai_call` row — so a computation is paid for once and reads stay
//! deterministic. Tokens only; there is no cost.
//!
//! - `classify(caller, text, labels)` and `score(caller, text, levels)`
//!   ask the `decide` role a typed question, and `decide(caller, state,
//!   questions)` asks it any set of them;
//! - `summarize(caller, text, focus)` runs on the `summarize` role;
//! - `extract(caller, instructions, text, schema)` asks the `main` role
//!   for JSON matching `schema`, and refuses a reply that doesn't.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_ai::client::{Answer, Question};
use oxplow_ai::config::Role;
use oxplow_db::{NewAiResult, SqliteAiResultStore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai_service::{AiService, AiServiceError, CallSite, ModelRequest};

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

/// One computation, before it runs: what it is (`name`, `args`), the
/// request it sends, on which role, for whom.
struct Op<'a> {
    name: &'static str,
    role: Role,
    caller: &'a str,
    args: Value,
    request: ModelRequest<'a>,
}

impl AiCompute {
    pub fn new(ai: Arc<AiService>, results: SqliteAiResultStore) -> Self {
        Self { ai, results }
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
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<(T, i64, i64, Option<i64>), AiComputeError>>,
    {
        let binding = self.ai.binding_for(op.role)?;
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
        let (value, input_tokens, output_tokens, ai_call_id) = compute(hash.clone()).await?;
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
                role: crate::ai_service::role_name(op.role),
                caller: op.caller.into(),
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

    /// Which of `labels` fits `text` (the `decide` role).
    pub async fn classify(
        &self,
        caller: &str,
        text: &str,
        labels: &[String],
    ) -> Result<Recorded<Classification>, AiComputeError> {
        let questions = BTreeMap::from([(
            "label".to_string(),
            Question::Choice {
                instructions: CLASSIFY_INSTRUCTIONS.into(),
                options: labels.to_vec(),
            },
        )]);
        let op = Op {
            name: "classify",
            role: Role::Decide,
            caller,
            args: json!({ "text": text, "labels": labels }),
            request: ModelRequest::Decide {
                state: text,
                questions: &questions,
            },
        };
        let questions = &questions;
        self.recorded(op, |hash| async move {
            let (decision, call) = self
                .ai
                .decide_as(Role::Decide, site(caller, &hash), text, questions)
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

    /// Where `text` sits on `levels`, lowest first (the `decide` role).
    pub async fn score(
        &self,
        caller: &str,
        text: &str,
        levels: &[String],
    ) -> Result<Recorded<Scored>, AiComputeError> {
        let questions = BTreeMap::from([(
            "score".to_string(),
            Question::Score {
                instructions: SCORE_INSTRUCTIONS.into(),
                levels: levels.to_vec(),
            },
        )]);
        let op = Op {
            name: "score",
            role: Role::Decide,
            caller,
            args: json!({ "text": text, "levels": levels }),
            request: ModelRequest::Decide {
                state: text,
                questions: &questions,
            },
        };
        let questions = &questions;
        self.recorded(op, |hash| async move {
            let (decision, call) = self
                .ai
                .decide_as(Role::Decide, site(caller, &hash), text, questions)
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

    /// Answers to typed `questions` about `state` (the `decide` role), by
    /// question name. A reply missing an answer is refused.
    pub async fn decide(
        &self,
        caller: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Recorded<BTreeMap<String, Answer>>, AiComputeError> {
        let op = Op {
            name: "decide",
            role: Role::Decide,
            caller,
            args: json!({ "state": state, "questions": questions }),
            request: ModelRequest::Decide { state, questions },
        };
        self.recorded(op, |hash| async move {
            let (decision, call) = self
                .ai
                .decide_as(Role::Decide, site(caller, &hash), state, questions)
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

    /// A summary of `text` (the `summarize` role), focused on `focus`.
    pub async fn summarize(
        &self,
        caller: &str,
        text: &str,
        focus: Option<&str>,
    ) -> Result<Recorded<String>, AiComputeError> {
        let system = crate::ai_service::summarize_system(focus);
        let op = Op {
            name: "summarize",
            role: Role::Summarize,
            caller,
            args: json!({ "text": text, "focus": focus }),
            request: ModelRequest::Complete {
                system: Some(&system),
                prompt: text,
                json: false,
            },
        };
        let system = &system;
        self.recorded(op, |hash| async move {
            let (c, call) = self
                .ai
                .complete_as(
                    Role::Summarize,
                    site(caller, &hash),
                    Some(system),
                    text,
                    false,
                )
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

    /// JSON matching `schema` from `text`, as `instructions` ask (the
    /// `main` role, in JSON mode). A reply that isn't JSON, or doesn't
    /// match, is refused and not recorded.
    pub async fn extract(
        &self,
        caller: &str,
        instructions: &str,
        text: &str,
        schema: &Value,
    ) -> Result<Recorded<Value>, AiComputeError> {
        let system =
            format!("{instructions}\n\nReply with JSON only, matching this JSON Schema:\n{schema}");
        let op = Op {
            name: "extract",
            role: Role::Main,
            caller,
            args: json!({ "instructions": instructions, "text": text, "schema": schema }),
            request: ModelRequest::Complete {
                system: Some(&system),
                prompt: text,
                json: true,
            },
        };
        let system = &system;
        self.recorded(op, |hash| async move {
            let validator = oxplow_domain::InputValidator::compile(schema).map_err(|e| {
                AiComputeError::BadOutput(format!("the schema doesn't compile: {e}"))
            })?;
            let (c, call) = self
                .ai
                .complete_as(Role::Main, site(caller, &hash), Some(system), text, true)
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
/// recording as `caller` (`collector:<owner>/<id>`). The script runs on a
/// worker thread outside the runtime; each call blocks it on the runtime
/// the oracle was made on.
pub struct CollectorOracle {
    compute: Arc<AiCompute>,
    caller: String,
    runtime: tokio::runtime::Handle,
}

impl CollectorOracle {
    /// Made inside the runtime its calls run on.
    pub fn new(compute: Arc<AiCompute>, caller: String) -> Self {
        Self {
            compute,
            caller,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    fn run<T: Serialize>(
        &self,
        f: impl std::future::Future<Output = Result<Recorded<T>, AiComputeError>>,
    ) -> Result<Value, String> {
        self.runtime
            .block_on(f)
            .map(|r| serde_json::to_value(r.value).expect("result serializes"))
            .map_err(|e| e.to_string())
    }
}

impl oxplow_script::AiOracle for CollectorOracle {
    fn classify(&self, text: &str, labels: &[String]) -> Result<Value, String> {
        self.run(self.compute.classify(&self.caller, text, labels))
    }

    fn score(&self, text: &str, levels: &[String]) -> Result<Value, String> {
        self.run(self.compute.score(&self.caller, text, levels))
    }

    fn summarize(&self, text: &str, focus: Option<&str>) -> Result<String, String> {
        self.run(self.compute.summarize(&self.caller, text, focus))
            .map(|v| v.as_str().unwrap_or_default().to_string())
    }

    fn extract(&self, instructions: &str, text: &str, schema: &Value) -> Result<Value, String> {
        self.run(
            self.compute
                .extract(&self.caller, instructions, text, schema),
        )
    }
}

fn site<'a>(caller: &'a str, hash: &'a str) -> CallSite<'a> {
    CallSite {
        caller,
        input_hash: Some(hash),
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
        crate::test_fixtures::approve_ai_providers(svc);
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
            .set_role(
                role,
                Some(RoleBinding {
                    provider: provider.into(),
                    model: "x".into(),
                }),
            )
            .unwrap();
    }

    fn chat(content: &str) -> Value {
        json!({ "choices": [{ "message": { "content": content } }], "usage": { "prompt_tokens": 7, "completion_tokens": 3 } })
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
        with_model(&fx.svc, Role::Summarize, chat("  A short summary.  ")).await;
        let first = fx
            .svc
            .ai_compute
            .summarize("test", "a long text", None)
            .await
            .unwrap();
        assert_eq!(first.value, "A short summary.");
        assert!(!first.cached);
        assert_eq!((first.input_tokens, first.output_tokens), (7, 3));
        let second = fx
            .svc
            .ai_compute
            .summarize("test", "a long text", None)
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
            .summarize("test", "a long text", Some("risks"))
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
        with_model(&fx.svc, Role::Decide, chat(&reply.to_string())).await;
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
                .decide("thread:thr1", "a diff", &questions)
                .await
                .unwrap();
            assert_eq!(out.cached, cached);
            assert_eq!(out.value["risky"], Answer::Noul { probability: 0.8 });
        }
        let recorded = calls(&fx.svc).await;
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].0, "thread:thr1");

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
            .decide("thread:thr1", "a diff", &more)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("`big`"), "{err}");
    }

    /// A result is the provider's as well as the model's: rebinding a role
    /// to another provider serving the same model name computes afresh.
    #[tokio::test]
    async fn another_provider_of_the_same_model_computes_afresh() {
        let fx = crate::test_fixtures::services_with_effort().await;
        with_provider(&fx.svc, Role::Summarize, "local", chat("local says")).await;
        let local = fx
            .svc
            .ai_compute
            .summarize("test", "t", None)
            .await
            .unwrap();
        assert_eq!(local.value, "local says");
        with_provider(&fx.svc, Role::Summarize, "hosted", chat("hosted says")).await;
        let hosted = fx
            .svc
            .ai_compute
            .summarize("test", "t", None)
            .await
            .unwrap();
        assert!(!hosted.cached);
        assert_eq!(hosted.value, "hosted says");
        fx.svc
            .ai
            .set_role(
                Role::Summarize,
                Some(RoleBinding {
                    provider: "local".into(),
                    model: "x".into(),
                }),
            )
            .unwrap();
        let again = fx
            .svc
            .ai_compute
            .summarize("test", "t", None)
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
            Role::Main,
            chat("```json\n{\"n\": \"three\"}\n```"),
        )
        .await;
        let schema = json!({ "type": "object", "required": ["n"], "properties": { "n": { "type": "integer" } } });
        let refused = fx
            .svc
            .ai_compute
            .extract("test", "Count them.", "one two three", &schema)
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
}
