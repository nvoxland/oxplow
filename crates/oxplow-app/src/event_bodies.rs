//! Reading an event's stored body — a tool call's input or output, a
//! prompt's text — for the
//! activity panel (a person, any stream) and the MCP `read_event_content`
//! tool (an agent, its own stream only). One routine for both, so the
//! scope and the size cap can't differ between them (tsk509).

use oxplow_domain::events::schema::ContentRef;
use oxplow_domain::{DomainError, EventId, StreamId};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Services;

/// The most of a body one read returns.
pub const MAX_READ_BYTES: usize = 64 * 1024;

/// Which body of the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventBodyKey {
    Input,
    Output,
    /// An `agent.prompt.submitted`'s text.
    Prompt,
}

impl EventBodyKey {
    fn field(self) -> &'static str {
        match self {
            EventBodyKey::Input => "input",
            EventBodyKey::Output => "output",
            EventBodyKey::Prompt => "prompt",
        }
    }
}

/// A body as read: UTF-8 (lossy) text, its full size, and whether what is
/// returned is less than the whole (capped at store or at read).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EventBody {
    pub text: String,
    pub size: u64,
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EventBodyError {
    #[error("no event `{0}`")]
    UnknownEvent(String),
    #[error("event `{0}` belongs to another stream")]
    OtherStream(String),
    #[error(transparent)]
    Storage(#[from] DomainError),
}

/// The `key` body of event `event_id`; `None` when the event has no such
/// body, or retention removed it. `scope` limits a reader to one stream's
/// events (an agent); a person reads any.
pub async fn read(
    svc: &Services,
    event_id: &str,
    key: EventBodyKey,
    scope: Option<StreamId>,
) -> Result<Option<EventBody>, EventBodyError> {
    let event = svc
        .event_log_store
        .get(EventId(event_id.to_string()))
        .await?
        .ok_or_else(|| EventBodyError::UnknownEvent(event_id.to_string()))?;
    if let Some(stream) = scope {
        if event.envelope.anchors.stream_id != Some(stream) {
            return Err(EventBodyError::OtherStream(event_id.to_string()));
        }
    }
    let Ok(content) =
        serde_json::from_value::<ContentRef>(event.envelope.payload[key.field()].clone())
    else {
        return Ok(None);
    };
    let Some(bytes) = oxplow_db::event_content_store::read(&svc.db, &content.hash).await? else {
        return Ok(None);
    };
    let end = oxplow_db::event_content_store::char_boundary_at_or_below(&bytes, MAX_READ_BYTES);
    Ok(Some(EventBody {
        text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
        size: content.size,
        truncated: content.truncated || end < bytes.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HookEnvelope, ToolDecision};
    use oxplow_domain::HookKind;

    /// An agent reads its own stream's bodies, capped, and never another
    /// stream's; a person reads any.
    #[tokio::test]
    async fn a_body_is_read_by_event_scoped_and_capped() {
        let f = crate::test_fixtures::services_with_effort().await;
        let big = "x".repeat(MAX_READ_BYTES * 2);
        f.svc
            .hook_ingest
            .ingest(HookEnvelope {
                kind: HookKind::PostToolUse,
                thread_id: Some(f.thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("s".into()),
                payload_json: serde_json::json!({
                    "tool_name": "Bash",
                    "tool_input": {"command": "ls"},
                    "tool_response": {"stdout": big},
                })
                .to_string(),
                prompt: None,
                decision: None::<ToolDecision>,
            })
            .await
            .unwrap();
        let event = f
            .svc
            .event_log_store
            .recent("agent.tool", Some(f.thread), None, 1)
            .await
            .unwrap()
            .remove(0);
        let id = event.envelope.id.as_str().to_string();
        let own = event.envelope.anchors.stream_id.unwrap();

        let input = read(&f.svc, &id, EventBodyKey::Input, Some(own))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(input.text, r#"{"command":"ls"}"#);
        assert!(!input.truncated);

        let output = read(&f.svc, &id, EventBodyKey::Output, None)
            .await
            .unwrap()
            .unwrap();
        assert!(output.truncated);
        assert_eq!(output.text.len(), MAX_READ_BYTES);
        assert!(output.size > MAX_READ_BYTES as u64);

        let other = StreamId::new(own.value() + 1);
        assert!(matches!(
            read(&f.svc, &id, EventBodyKey::Input, Some(other)).await,
            Err(EventBodyError::OtherStream(_))
        ));
        assert!(matches!(
            read(&f.svc, "evt-none", EventBodyKey::Input, None).await,
            Err(EventBodyError::UnknownEvent(_))
        ));
    }

    /// Every prompt's text is kept with its event — a re-prompt inside an
    /// open turn too, which the turn row (holding the first) never saw.
    #[tokio::test]
    async fn a_reprompts_text_is_kept_with_its_event() {
        let f = crate::test_fixtures::services_with_effort().await;
        for text in ["first ask", "and then this"] {
            f.svc
                .hook_ingest
                .ingest(HookEnvelope {
                    kind: HookKind::UserPromptSubmit,
                    thread_id: Some(f.thread),
                    stream_id: None,
                    agent_session_id: None,
                    session_id: Some("s".into()),
                    payload_json: "{}".into(),
                    prompt: Some(text.into()),
                    decision: None,
                })
                .await
                .unwrap();
        }
        let prompts = f
            .svc
            .event_log_store
            .recent("agent.prompt", Some(f.thread), None, 10)
            .await
            .unwrap();
        let reprompt = &prompts[0];
        assert_eq!(reprompt.envelope.payload["reprompt"], true);
        let body = read(
            &f.svc,
            reprompt.envelope.id.as_str(),
            EventBodyKey::Prompt,
            None,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(body.text, "and then this");
    }
}
