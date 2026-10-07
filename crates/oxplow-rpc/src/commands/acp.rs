//! Cores for the `acp` command module: an ACP thread's session (tsk281).
//!
//! Every command here is UI-only (`ui(...)` in the surface-parity table),
//! so none can become an MCP tool. In particular `acp_prompt` is the
//! prompt box's Enter and nothing else: an agent can't prompt an agent,
//! and oxplow never prompts one on its own (see
//! `oxplow_app::acp::human_prompt`).

use std::sync::Arc;

use oxplow_app::acp::host::ServicesAcpHost;
use oxplow_app::acp::manager::{AcpSnapshot, Launch};
use oxplow_app::acp::session::{AcpError, SessionSpec};
use oxplow_app::acp::wire::McpHttp;
use oxplow_app::acp::{agents, manager::AcpManager};
use oxplow_app::Services;
use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::{AgentKind, ThreadId};

use crate::error::IpcError;
use crate::RpcContext;

fn acp_err(e: AcpError) -> IpcError {
    IpcError::invalid(e.to_string())
}

fn snapshot(acp: &AcpManager, thread: &ThreadId, since: u64) -> Result<AcpSnapshot, IpcError> {
    acp.transcript(thread, since)
        .ok_or_else(|| acp_err(AcpError::NotOpen))
}

/// Start the thread's ACP agent (or keep the running one) and return the
/// session. Loads the thread's last session when the agent supports it.
pub async fn acp_open_session(
    ctx: &RpcContext,
    thread_id: ThreadId,
) -> Result<AcpSnapshot, IpcError> {
    let svc = &ctx.services;
    if svc.acp.is_open(&thread_id) {
        return snapshot(&svc.acp, &thread_id, 0);
    }
    let thread = svc
        .thread_store
        .get(&thread_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(IpcError::not_found)?;
    let name = match (thread.agent, thread.acp_agent.as_deref()) {
        (AgentKind::Acp, Some(name)) => name.to_string(),
        _ => return Err(IpcError::invalid("this thread doesn't run an ACP agent")),
    };
    let stream = svc
        .stream_store
        .get(&thread.stream_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(IpcError::not_found)?;
    let config = svc
        .config
        .read()
        .map(|c| c.clone())
        .map_err(|_| IpcError::internal("config lock poisoned"))?;
    let project_dir = svc.layout.project_dir.clone();
    let (agent, source) = agents::find(&config, &name)
        .ok_or_else(|| IpcError::invalid(format!("no ACP agent named '{name}' is configured")))?;
    // Checked where it runs: script args are read from the stream's worktree.
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    if !agents::may_start(&svc.approvals, &project_dir, &cwd, &agent, source) {
        return Err(IpcError::invalid(format!(
            "ACP agent '{name}' is a project program; approve it in Settings → Data → Programs first"
        )));
    }
    let program = agents::resolve_command(&project_dir, &agent.command).ok_or_else(|| {
        IpcError::invalid(format!(
            "'{}' isn't installed (ACP agent '{name}')",
            agent.command
        ))
    })?;

    let mcp = match &ctx.plugin_runtime {
        Some(rt) => vec![McpHttp {
            name: "oxplow".into(),
            url: rt.mcp_endpoint_url.clone(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", rt.hook_token)),
                ("X-Oxplow-Thread".into(), thread.id.to_string()),
                ("X-Oxplow-Stream".into(), stream.id.to_string()),
            ],
        }],
        None => vec![],
    };
    let system_prompt = oxplow_app::agent_prompt::assemble_acp_system_prompt(
        &project_dir,
        &config,
        &stream,
        Some(&thread),
        &oxplow_app::capabilities::agent_text(ctx),
    );
    let spec = SessionSpec {
        thread_id,
        agent: name,
        cwd: std::path::PathBuf::from(&stream.worktree_path),
        mcp,
        resume_session_id: Some(thread.resume_session_id.clone()).filter(|s| !s.is_empty()),
        system_prompt: Some(system_prompt),
        system_prompt_via_meta: agents::system_prompt_via_meta(&agent),
    };
    let launch = Launch {
        program: program.into(),
        args: agent.args.clone(),
        env: agent.env.clone().into_iter().collect(),
    };
    let host = Arc::new(ServicesAcpHost::new(svc, Some(stream.id)));
    svc.acp.open(host, spec, launch).await.map_err(acp_err)?;
    snapshot(&svc.acp, &thread_id, 0)
}

/// What the person typed in the prompt box. The one path by which a
/// prompt reaches an ACP agent.
pub async fn acp_prompt(svc: &Services, thread_id: ThreadId, text: String) -> Result<(), IpcError> {
    svc.acp
        .submit_human_prompt(&thread_id, text)
        .await
        .map_err(acp_err)
}

/// Stop the running turn (open permission cards are cancelled).
pub async fn acp_cancel(svc: &Services, thread_id: ThreadId) -> Result<(), IpcError> {
    svc.acp.cancel(&thread_id).map_err(acp_err)
}

/// Answer a permission card; no `option_id` cancels it.
pub async fn acp_respond_permission(
    svc: &Services,
    thread_id: ThreadId,
    request_id: String,
    option_id: Option<String>,
) -> Result<(), IpcError> {
    svc.acp
        .respond_permission(&thread_id, request_id, option_id)
        .await
        .map_err(acp_err)
}

/// The session and the transcript items changed after `since_seq` (0 for
/// all). `None` when the thread has no session in this daemon.
pub async fn acp_transcript(
    svc: &Services,
    thread_id: ThreadId,
    since_seq: u64,
) -> Result<Option<AcpSnapshot>, IpcError> {
    Ok(svc.acp.transcript(&thread_id, since_seq))
}

/// Stop the session and its agent process.
pub async fn acp_close_session(svc: &Services, thread_id: ThreadId) -> Result<(), IpcError> {
    svc.acp.close(&thread_id).map_err(acp_err)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    #[tokio::test]
    async fn without_a_session_prompting_is_refused_and_the_transcript_is_empty() {
        let (ctx, _dir) = crate::test_support::services();
        let err = crate::dispatch(
            "acp_prompt",
            json!({"threadId": "thr1", "text": "hi"}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("no ACP session"), "{}", err.message);
        let t = crate::dispatch(
            "acp_transcript",
            json!({"threadId": "thr1", "sinceSeq": 0}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(t, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn opening_a_terminal_agent_thread_is_refused() {
        let (ctx, _dir) = crate::test_support::services();
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let t = crate::dispatch(
            "run_command",
            json!({"name": "oxplow.thread.create", "input": {
                "stream": oxplow_domain::refs::build::stream_ref(stream.id),
                "title": "c", "agent": "claude",
            }, "confirmed": false}),
            &ctx,
        )
        .await
        .unwrap()["result"]
            .clone();
        let err = crate::dispatch("acp_open_session", json!({"threadId": t["id"]}), &ctx)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("doesn't run an ACP agent"),
            "{}",
            err.message
        );
    }
}
