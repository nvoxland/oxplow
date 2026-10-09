//! Reading what one of oxplow's own model calls was asked and answered
//! (`ai_call.request_hash` / `response_hash`, bodies in `event_content`
//! under `ai`): for a person any call (RPC `read_ai_call`), for an agent
//! only the calls it asked for (MCP `read_ai_call`, recorded as its
//! thread). Capped like an event's body. See `.context/ai-providers.md`.

use oxplow_domain::DomainError;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::event_bodies::{EventBody, MAX_READ_BYTES};
use crate::Services;

/// Which body of the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiCallBody {
    /// What the model was asked: `{ system, prompt, json }` or `{ state,
    /// questions }`.
    Request,
    /// What it answered: `{ text }` or `{ answers }`.
    Response,
}

#[derive(Debug, thiserror::Error)]
pub enum AiCallBodyError {
    #[error("no model call {0}")]
    UnknownCall(i64),
    #[error("model call {0} was asked for by another caller")]
    OtherCaller(i64),
    #[error(transparent)]
    Storage(#[from] DomainError),
}

/// The `body` of model call `id`; `None` when it has none (a failed
/// call's response) or retention removed it. `scope` limits a reader to
/// the calls recorded as that caller (an agent's thread ref); a person
/// reads any.
pub async fn read(
    svc: &Services,
    id: i64,
    body: AiCallBody,
    scope: Option<&str>,
) -> Result<Option<EventBody>, AiCallBodyError> {
    let row = svc
        .db
        .read(move |c| {
            c.query_row(
                "SELECT caller, request_hash, response_hash FROM ai_call WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await?;
    let (caller, request, response) = row.ok_or(AiCallBodyError::UnknownCall(id))?;
    if scope.is_some_and(|s| s != caller) {
        return Err(AiCallBodyError::OtherCaller(id));
    }
    let hash = match body {
        AiCallBody::Request => request,
        AiCallBody::Response => response,
    };
    let Some(hash) = hash else {
        return Ok(None);
    };
    let Some(bytes) = oxplow_db::event_content_store::read(&svc.db, &hash).await? else {
        return Ok(None);
    };
    let end = oxplow_db::event_content_store::char_boundary_at_or_below(&bytes, MAX_READ_BYTES);
    Ok(Some(EventBody {
        text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
        size: bytes.len() as u64,
        truncated: end < bytes.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_service::{ProviderConfig, Role, RoleBinding};

    /// A person reads any call's request and response; an agent only the
    /// calls recorded as its own thread; a failed call has no response.
    #[tokio::test]
    async fn a_calls_bodies_are_read_scoped_to_their_caller() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let (base, _) = oxplow_ai_fake::mock(
            "/chat/completions",
            200,
            serde_json::json!({ "choices": [{ "message": { "content": "short" } }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }),
        )
        .await;
        crate::test_fixtures::approve_ai_providers(&fx.svc);
        fx.svc
            .ai
            .save_provider(
                ProviderConfig {
                    id: "m".into(),
                    kind: "openai_compatible".into(),
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        fx.svc
            .ai
            .set_role(
                Role::Summarize,
                Some(RoleBinding {
                    provider: "m".into(),
                    model: "x".into(),
                }),
            )
            .unwrap();
        let mine = "thread:thr1";
        let id = fx
            .svc
            .ai_compute
            .summarize(mine, "a long log", None)
            .await
            .unwrap()
            .ai_call_id
            .unwrap();
        let request = read(&fx.svc, id, AiCallBody::Request, Some(mine))
            .await
            .unwrap()
            .unwrap();
        assert!(
            request.text.contains(r#""prompt":"a long log""#),
            "{}",
            request.text
        );
        let response = read(&fx.svc, id, AiCallBody::Response, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.text, r#"{"text":"short"}"#);
        assert!(matches!(
            read(&fx.svc, id, AiCallBody::Request, Some("thread:thr2")).await,
            Err(AiCallBodyError::OtherCaller(_))
        ));
        assert!(matches!(
            read(&fx.svc, id + 100, AiCallBody::Request, None).await,
            Err(AiCallBodyError::UnknownCall(_))
        ));
    }
}
