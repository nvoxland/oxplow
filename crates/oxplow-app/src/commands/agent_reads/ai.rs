//! `ai.compute` operations an agent calls: a typed question about some
//! text (`oxplow.ai.decide`) and a summary (`oxplow.ai.summarize`). Each
//! is a recorded computation (`ai_result`, `v_ai_call`) whose caller is
//! the asking thread's ref; the call is paid with the person's key, so a
//! caller with no thread behind it asks nothing.

use std::collections::BTreeMap;

use oxplow_domain::{Actor, CommandError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, ServicesSlot};
use super::super::HandlerOutput;
use super::ids::{read_op, result_of};
use crate::ai_compute::AiComputeError;
use crate::ai_service::{AiServiceError, Question, Role};

/// The most text an agent's model call takes (`decide`'s `state`,
/// `summarize`'s `text`), in characters: each call is paid with the
/// person's key.
pub const AI_TEXT_MAX: usize = 100_000;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AiQuestionParam {
    /// `noul` (yes/no: returns the probability of yes), `choice` (pick one
    /// of `options`) or `score` (place it on the ordered `levels`).
    #[serde(rename = "type")]
    pub kind: String,
    /// The question, e.g. "Does this change alter public behavior?".
    pub instructions: String,
    /// For `choice`: the options.
    pub options: Option<Vec<String>>,
    /// For `score`: the levels, lowest first.
    pub levels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AiDecideParams {
    /// What the questions are about: a diff, a description, some text.
    pub state: String,
    /// Questions by a short name you choose (e.g. `risky`).
    pub questions: BTreeMap<String, AiQuestionParam>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AiSummarizeParams {
    pub text: String,
    /// What to focus on, e.g. "risks" or "what changed for users".
    pub focus: Option<String>,
}

/// What an agent's model call is recorded as: the asking thread's ref.
fn ai_caller(actor: &Actor) -> Result<String, CommandError> {
    match actor {
        Actor::Agent {
            thread_id: Some(thread),
            ..
        } => Ok(oxplow_domain::uri::build::thread_uri(*thread)),
        _ => Err(CommandError::Invalid {
            field: None,
            message: "this connection carries no thread identity (it didn't come through an \
                      agent session's bearer), so it may not call a model; oxplow's own harness \
                      configs connect with one — reconnect through one"
                .into(),
        }),
    }
}

fn check_ai_text(field: &str, text: &str) -> Result<(), CommandError> {
    let n = text.chars().count();
    if n > AI_TEXT_MAX {
        return Err(invalid(
            &format!("/{field}"),
            format!("`{field}` is {n} characters; a model call takes at most {AI_TEXT_MAX}"),
        ));
    }
    Ok(())
}

fn ai_error(e: AiServiceError) -> CommandError {
    match e {
        AiServiceError::Secret(_) => failed(e),
        _ => CommandError::Invalid {
            field: None,
            message: e.to_string(),
        },
    }
}

/// A recorded computation's failure: the model's, as [`ai_error`]; an
/// unusable answer the agent can retry; storing it is internal.
fn compute_error(e: AiComputeError) -> CommandError {
    match e {
        AiComputeError::Ai(e) => ai_error(e),
        e @ AiComputeError::BadOutput(_) => CommandError::Invalid {
            field: None,
            message: e.to_string(),
        },
        e @ AiComputeError::Storage(_) => failed(e),
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        read_op(
            "ai.compute",
            "decide",
            services.clone(),
            |svc, actor, p: AiDecideParams| async move {
                let caller = ai_caller(&actor)?;
                check_ai_text("state", &p.state)?;
                let mut questions = BTreeMap::new();
                for (name, q) in p.questions {
                    let bad = |m: &str| {
                        invalid(
                            &format!("/questions/{name}"),
                            format!("question `{name}`: {m}"),
                        )
                    };
                    let question = match q.kind.as_str() {
                        "noul" => Question::Noul {
                            instructions: q.instructions,
                        },
                        "choice" => Question::Choice {
                            instructions: q.instructions,
                            options: q
                                .options
                                .filter(|o| !o.is_empty())
                                .ok_or_else(|| bad("a choice needs options"))?,
                        },
                        "score" => Question::Score {
                            instructions: q.instructions,
                            levels: q
                                .levels
                                .filter(|l| !l.is_empty())
                                .ok_or_else(|| bad("a score needs levels"))?,
                        },
                        other => {
                            return Err(bad(&format!("type `{other}` isn't noul, choice or score")))
                        }
                    };
                    questions.insert(name, question);
                }
                let decision = svc
                    .ai_compute
                    .decide(caller.as_str(), &Role::DECIDE, &p.state, &questions)
                    .await
                    .map_err(compute_error)?;
                result_of(json!({ "answers": decision.value, "cached": decision.cached }))
            },
        ),
        read_op(
            "ai.compute",
            "summarize",
            services,
            |svc, actor, p: AiSummarizeParams| async move {
                let caller = ai_caller(&actor)?;
                check_ai_text("text", &p.text)?;
                let summary = svc
                    .ai_compute
                    .summarize(
                        caller.as_str(),
                        &Role::SUMMARIZE,
                        &p.text,
                        p.focus.as_deref(),
                    )
                    .await
                    .map_err(compute_error)?;
                Ok(HandlerOutput {
                    result: Value::String(summary.value),
                    ..HandlerOutput::default()
                })
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// With no thread behind the call — or no model assigned to the role,
    /// or too much text — the model is never asked, and the refusal says
    /// why.
    #[tokio::test]
    async fn a_model_is_asked_only_for_a_thread_with_a_model_and_bounded_text() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let anonymous = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        };
        let thread = Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(1)),
            stream_id: None,
        };
        let risky = json!({ "risky": { "type": "noul", "instructions": "Is it risky?" } });
        let run = |actor: &Actor, id: &str, input: Value| {
            let svc = svc.clone();
            let (actor, id) = (actor.clone(), id.to_string());
            async move { svc.commands.run(&actor, &id, input, false).await }
        };
        let err = run(
            &anonymous,
            "oxplow.ai.decide",
            json!({ "state": "a diff", "questions": risky }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        let err = run(
            &thread,
            "oxplow.ai.decide",
            json!({ "state": "x".repeat(AI_TEXT_MAX + 1), "questions": risky }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("characters"), "{err}");
        let err = run(
            &thread,
            "oxplow.ai.decide",
            json!({ "state": "s", "questions": { "x": { "type": "maybe", "instructions": "?" } } }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("maybe"), "{err}");
        let err = run(&thread, "oxplow.ai.summarize", json!({ "text": "t" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("summarize"), "{err}");
    }
}
