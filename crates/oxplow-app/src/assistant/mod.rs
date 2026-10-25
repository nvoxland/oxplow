//! The Assistant: a short-lived agent in oxplow's own panel, for
//! questions and small scoped tasks (`.context/ai-providers.md` "The
//! Assistant"). It runs on Rig's agent loop (`rig-agent`) over the
//! `assistant` role's model, with bus commands as its tools. Sessions live
//! in memory, by id (`ast<n>`), and are gone on restart. It is not an
//! agent session: it never touches the thread's sessions, its context
//! basket or its handoffs (it may send one, when the person asks).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oxplow_domain::{StreamId, ThreadId};
use serde::{Deserialize, Serialize};

use crate::chat::transcript::TranscriptItem;
use crate::Services;

pub mod prompt;
mod session;
pub mod tools;

#[cfg(test)]
mod tests;

use session::Session;

/// What the Assistant was opened on: a page, or a selection of text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AssistantContext {
    /// A page, by its URI.
    Page { uri: String, label: Option<String> },
    /// Selected text, and where it came from when known.
    Selection {
        uri: Option<String>,
        text: String,
        label: Option<String>,
    },
}

/// Whether a turn is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AssistantStatus {
    Idle,
    Running,
}

/// A provider's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantModel {
    pub provider: String,
    pub model: String,
}

/// A model the person may pick, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantModelOption {
    pub provider: String,
    pub model: String,
    pub label: String,
}

/// `assistant_set`'s `model` on the wire: a model, or `"default"` (back to
/// the `assistant` role's binding). `api.ts`'s `assistantSet` sends
/// `"default"` for its `model: null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(untagged)]
pub enum AssistantModelChoice {
    Default(RoleDefault),
    Model(AssistantModel),
}

/// The `"default"` of [`AssistantModelChoice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum RoleDefault {
    #[serde(rename = "default")]
    Default,
}

/// Why the Assistant can't answer now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AssistantProblemKind {
    /// No model is bound to the `assistant` role or to Main.
    NoModel,
    /// The model couldn't be used (its key, its provider, a restriction).
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantProblem {
    pub kind: AssistantProblemKind,
    pub message: String,
}

/// A session as the panel reads it: the items changed after `since`.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSnapshot {
    pub status: AssistantStatus,
    /// Items created or changed after `since`, oldest first; upsert by `id`.
    pub items: Vec<TranscriptItem>,
    /// The highest `seq` so far: ask `since` it next time.
    #[specta(type = oxplow_domain::JsNumber)]
    pub seq: u64,
    /// The model a prompt runs on: the person's pick, else the role's.
    pub model: Option<AssistantModel>,
    pub models: Vec<AssistantModelOption>,
    pub allow_changes: bool,
    pub problem: Option<AssistantProblem>,
}

/// What `assistant_open` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantOpened {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssistantError {
    #[error("there is no Assistant session `{0}`")]
    NotFound(String),
    #[error("the Assistant is still answering: stop it first, or wait")]
    TurnInFlight,
    #[error("{0}")]
    NoModel(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Failed(String),
}

/// The open sessions (`Services.assistant`).
#[derive(Default)]
pub struct AssistantManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next: AtomicU64,
}

impl AssistantManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, AssistantError> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| AssistantError::NotFound(id.to_string()))
    }

    /// Open a session in `stream` (and `thread`, where the person is), on
    /// what they asked about. Opens with no model too: the snapshot then
    /// says why (`problem`).
    pub async fn open(
        &self,
        svc: &Services,
        stream: StreamId,
        thread: Option<ThreadId>,
        context: Option<AssistantContext>,
    ) -> Result<AssistantOpened, AssistantError> {
        let id = format!("ast{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let place = session::place(svc, stream, thread).await?;
        let session = Arc::new(Session::new(
            id.clone(),
            stream,
            thread,
            context,
            place,
            svc.events.clone(),
        ));
        session.refresh(svc).await;
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), session);
        Ok(AssistantOpened { id })
    }

    /// Run one turn on what the person typed. A prompt while a turn runs is
    /// refused.
    pub async fn prompt(&self, svc: &Services, id: &str, text: &str) -> Result<(), AssistantError> {
        self.get(id)?.prompt(svc, text).await
    }

    /// Stop the running turn, if one is.
    pub fn cancel(&self, id: &str) -> Result<(), AssistantError> {
        self.get(id)?.cancel();
        Ok(())
    }

    /// Pick the model (`Default`: back to the role's) and allow or forbid
    /// changes; `None` leaves each as it is.
    pub async fn set(
        &self,
        svc: &Services,
        id: &str,
        model: Option<AssistantModelChoice>,
        allow_changes: Option<bool>,
    ) -> Result<(), AssistantError> {
        self.get(id)?.set(svc, model, allow_changes).await;
        Ok(())
    }

    /// Answer approval card `ask`: allow its call, or decline it.
    pub fn answer(&self, id: &str, ask: &str, allow: bool) -> Result<(), AssistantError> {
        self.get(id)?.answer(ask, allow)
    }

    /// The session, with the items changed after `since`.
    pub fn snapshot(&self, id: &str, since: u64) -> Result<AssistantSnapshot, AssistantError> {
        Ok(self.get(id)?.snapshot(since))
    }

    /// Continue in a thread's agent: hand it a condensed transcript
    /// (`oxplow.handoff.send`, as the person, through this session). The
    /// command's result.
    pub async fn handoff(
        &self,
        svc: &Services,
        id: &str,
        to_thread: &str,
    ) -> Result<serde_json::Value, AssistantError> {
        self.get(id)?.handoff(svc, to_thread).await
    }

    /// Close the session, stopping its turn.
    pub fn close(&self, id: &str) -> Result<(), AssistantError> {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
            .ok_or_else(|| AssistantError::NotFound(id.to_string()))?;
        session.close();
        Ok(())
    }
}
