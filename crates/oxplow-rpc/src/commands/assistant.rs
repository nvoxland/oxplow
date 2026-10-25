//! Cores for the Assistant's panel (`oxplow_app::assistant`): open a
//! session, prompt it, stop it, pick its model, allow changes, answer its
//! approval cards, read its transcript, continue in a thread, close it.
//! UI-only: the prompt is the person's Enter, never an agent's (an agent
//! has its own session). `assistantChanged { id, seq }` on `/events` says
//! when to read again. See `.context/ai-providers.md` "The Assistant".

use oxplow_app::assistant::{
    AssistantContext, AssistantError, AssistantModelChoice, AssistantOpened, AssistantSnapshot,
};
use oxplow_app::Services;
use oxplow_domain::{Json, StreamId, ThreadId};

use crate::error::IpcError;

impl From<AssistantError> for IpcError {
    fn from(e: AssistantError) -> Self {
        let code = match e {
            AssistantError::NotFound(_) => "NOT_FOUND",
            AssistantError::TurnInFlight => "BUSY",
            AssistantError::NoModel(_) => "NO_MODEL",
            AssistantError::Invalid(_) => "INVALID",
            AssistantError::Failed(_) => "INTERNAL",
        };
        let mut out = IpcError::invalid(e.to_string());
        out.code = code.into();
        out
    }
}

/// Open a session in `stream` (and `thread`) on what was asked about. It
/// opens with no model too; its snapshot's `problem` says why.
pub async fn assistant_open(
    svc: &Services,
    stream: StreamId,
    thread: Option<ThreadId>,
    context: Option<AssistantContext>,
) -> Result<AssistantOpened, IpcError> {
    Ok(svc.assistant.open(svc, stream, thread, context).await?)
}

/// Send what the person typed. Refused (`BUSY`) while a turn runs, and
/// (`NO_MODEL`) with no model bound.
pub async fn assistant_prompt(svc: &Services, id: String, text: String) -> Result<(), IpcError> {
    Ok(svc.assistant.prompt(svc, &id, &text).await?)
}

/// Stop the running turn.
pub async fn assistant_cancel(svc: &Services, id: String) -> Result<(), IpcError> {
    Ok(svc.assistant.cancel(&id)?)
}

/// Pick the model (`"default"`: back to the role's) and allow or forbid
/// changes; `null` leaves each as it is.
pub async fn assistant_set(
    svc: &Services,
    id: String,
    model: Option<AssistantModelChoice>,
    allow_changes: Option<bool>,
) -> Result<(), IpcError> {
    Ok(svc.assistant.set(svc, &id, model, allow_changes).await?)
}

/// Answer approval card `ask` (its `requestId`): allow or decline.
pub async fn assistant_answer(
    svc: &Services,
    id: String,
    ask: String,
    allow: bool,
) -> Result<(), IpcError> {
    Ok(svc.assistant.answer(&id, &ask, allow)?)
}

/// The session, with the transcript items changed after `since`.
pub async fn assistant_snapshot(
    svc: &Services,
    id: String,
    since: u32,
) -> Result<AssistantSnapshot, IpcError> {
    Ok(svc.assistant.snapshot(&id, u64::from(since))?)
}

/// Continue in thread `to_thread` (`thr3`): hand it the condensed
/// transcript. The handoff command's result.
pub async fn assistant_handoff(
    svc: &Services,
    id: String,
    to_thread: String,
) -> Result<Json, IpcError> {
    Ok(Json(svc.assistant.handoff(svc, &id, &to_thread).await?))
}

/// Close the session, stopping its turn.
pub async fn assistant_close(svc: &Services, id: String) -> Result<(), IpcError> {
    Ok(svc.assistant.close(&id)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    /// The panel's calls route: open with no model, read the snapshot
    /// (its problem), refuse a prompt, close.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_assistant_ipcs_route() {
        let (ctx, _dir) = services().await;
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let opened = crate::dispatch(
            "assistant_open",
            json!({
                "stream": stream.id.to_string(),
                "thread": null,
                "context": { "kind": "selection", "text": "fn main() {}", "label": "main.rs" },
            }),
            &ctx,
        )
        .await
        .unwrap();
        let id = opened["id"].as_str().unwrap().to_string();
        let snap = crate::dispatch("assistant_snapshot", json!({ "id": id, "since": 0 }), &ctx)
            .await
            .unwrap();
        assert_eq!(snap["status"], "idle");
        assert_eq!(snap["problem"]["kind"], "no_model");
        assert_eq!(snap["allowChanges"], false);
        let err = crate::dispatch("assistant_prompt", json!({ "id": id, "text": "hi" }), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NO_MODEL", "{}", err.message);
        crate::dispatch(
            "assistant_set",
            json!({ "id": id, "model": "default", "allowChanges": true }),
            &ctx,
        )
        .await
        .unwrap();
        let snap = crate::dispatch("assistant_snapshot", json!({ "id": id, "since": 0 }), &ctx)
            .await
            .unwrap();
        assert_eq!(snap["allowChanges"], true);
        crate::dispatch("assistant_close", json!({ "id": id }), &ctx)
            .await
            .unwrap();
        let err = crate::dispatch("assistant_snapshot", json!({ "id": id, "since": 0 }), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
    }
}
