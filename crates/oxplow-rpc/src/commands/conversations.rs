//! Cores for the `conversations` command module — the saved conversations
//! an agent session's harness keeps for its worktree, for the person to
//! choose one to resume or fork (`.context/terminal.md` "Choosing another
//! conversation"). Read-only: choosing is `oxplow.agent_session.resume_conversation`.

use oxplow_app::Services;
use oxplow_domain::agent::harness::{ConversationQuery, HarnessError, NativeConversation};
use oxplow_domain::stores::{AgentSessionStore as _, StreamStore as _, ThreadStore as _};
use oxplow_domain::AgentSessionId;
use serde::Serialize;

use crate::error::IpcError;

/// The most conversations listed.
const LIMIT: usize = 50;

/// One of a harness's saved conversations, with where it stands.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ListedConversation {
    pub conversation: NativeConversation,
    /// The session is on it now.
    pub current: bool,
    /// The other open session on it (`agent_session:ses4`), if one is:
    /// resuming it there is refused.
    pub open_in: Option<String>,
}

/// The saved conversations of `session_id`'s harness begun in its
/// stream's worktree, newest first. A harness that can't list them says
/// so, as does one that couldn't now.
pub async fn list_native_conversations(
    svc: &Services,
    session_id: String,
) -> Result<Vec<ListedConversation>, IpcError> {
    let id = AgentSessionId::try_from_str(&session_id)
        .ok_or_else(|| IpcError::invalid(format!("{session_id:?} is not an agent session id")))?;
    let session = svc
        .agent_session_store
        .get(&id)
        .await?
        .ok_or_else(|| IpcError::invalid(format!("no agent session {session_id}")))?;
    let thread = svc
        .thread_store
        .get(&session.thread_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let stream = svc
        .stream_store
        .get(&thread.stream_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let harness = svc
        .harnesses
        .get(session.harness.as_str())
        .map_err(|e| IpcError::invalid(e.to_string()))?;
    let query = ConversationQuery {
        cwd: std::path::PathBuf::from(&stream.worktree_path),
        // Its saved conversations are on the machine it runs on.
        home: (stream.host == oxplow_domain::HostId::LOCAL)
            .then(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
            .flatten(),
        search_path: oxplow_app::agent_path::search_path(),
        limit: LIMIT,
    };
    let found = harness.conversations(&query).await.map_err(|e| match e {
        HarnessError::Unsupported { .. } => IpcError::invalid(e.to_string()),
        other => IpcError::internal(other.to_string()),
    })?;
    let host = session.host.clone();
    let ids: Vec<String> = found.iter().map(|c| c.id.clone()).collect();
    let open_in: Vec<Option<String>> = svc
        .db
        .read(move |c| {
            ids.iter()
                .map(|native| {
                    Ok(oxplow_db::agent_session_store::bound_elsewhere_tx(
                        c,
                        native,
                        host.as_deref(),
                        id,
                    )?
                    .map(|s| oxplow_domain::refs::build::agent_session_ref(s.id)))
                })
                .collect()
        })
        .await?;
    Ok(found
        .into_iter()
        .zip(open_in)
        .map(|(conversation, open_in)| ListedConversation {
            current: conversation.id == session.resume_session_id,
            conversation,
            open_in,
        })
        .collect())
}

/// Name the session's conversation in its harness's own UI (Codex's
/// thread list), when its harness can (feature `native_rename`) and its
/// observer is running. A person's: the desktop's Rename calls it after
/// `oxplow.agent_session.rename` set oxplow's alias, so a failure here
/// leaves the alias saved and says the native name wasn't taken.
pub async fn rename_native_conversation(
    svc: &Services,
    session_id: String,
    name: String,
) -> Result<(), IpcError> {
    let id = AgentSessionId::try_from_str(&session_id)
        .ok_or_else(|| IpcError::invalid(format!("{session_id:?} is not an agent session id")))?;
    let session = svc
        .agent_session_store
        .get(&id)
        .await?
        .ok_or_else(|| IpcError::invalid(format!("no agent session {session_id}")))?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(IpcError::invalid("name the conversation".to_string()));
    }
    let can = svc
        .capabilities
        .get("agent_harness", &session.harness)
        .is_some_and(|i| i.features.get("native_rename").and_then(|v| v.as_bool()) == Some(true));
    if !can {
        return Err(IpcError::invalid(format!(
            "`{}` can't rename its conversation; Rename sets oxplow's name for the session only",
            session.harness
        )));
    }
    svc.chat
        .rename_native(&id, name)
        .await
        .map_err(|e| match e {
            oxplow_app::chat::session::ChatError::NotOpen => IpcError::invalid(
                "the session's conversation isn't being observed right now; open its tab and try again"
                    .to_string(),
            ),
            other => IpcError::internal(other.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A session whose harness can't list its conversations says so; an
    /// unknown session is refused.
    #[tokio::test]
    async fn a_harness_without_a_list_says_so() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch(
            "list_native_conversations",
            json!({ "sessionId": "ses999" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    async fn session_of(ctx: &crate::RpcContext, harness: &str) -> String {
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a writer thread");
        let out = crate::dispatch(
            "run_command",
            json!({ "id": "oxplow.agent_session.open", "input": {
                "thread": oxplow_domain::refs::build::thread_ref(thread),
                "harness": harness,
            }, "confirmed": false }),
            ctx,
        )
        .await
        .unwrap();
        out["result"]["id"].as_str().unwrap().to_string()
    }

    /// Only a harness that declares `native_rename` renames its
    /// conversation; the others say Rename is oxplow's name only; Codex's
    /// needs its observer running; a name is required.
    #[tokio::test]
    async fn a_native_rename_needs_the_feature_a_name_and_a_running_observer() {
        let (ctx, _dir) = crate::test_support::services().await;
        let rename = |session: &str, name: &str| {
            crate::dispatch(
                "rename_native_conversation",
                json!({ "sessionId": session, "name": name }),
                &ctx,
            )
        };
        let claude = session_of(&ctx, "claude").await;
        let err = rename(&claude, "x").await.unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(err.message.contains("can't rename"), "{}", err.message);
        let codex = session_of(&ctx, "codex").await;
        let err = rename(&codex, "  ").await.unwrap_err();
        assert!(
            err.message.contains("name the conversation"),
            "{}",
            err.message
        );
        let err = rename(&codex, "Login fix").await.unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("isn't being observed"),
            "{}",
            err.message
        );
        let err = rename("ses9999", "x").await.unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
