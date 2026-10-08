//! Cores for the `acp` command module: an ACP agent session's agent.
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
use oxplow_domain::agent::acp_adapter::SystemPromptVia;
use oxplow_domain::agent::harness::LaunchSpec;
use oxplow_domain::agent_session::SessionKind;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::AgentSessionId;

use crate::error::IpcError;
use crate::RpcContext;

fn acp_err(e: AcpError) -> IpcError {
    IpcError::invalid(e.to_string())
}

fn snapshot(
    acp: &AcpManager,
    session: &AgentSessionId,
    since: u64,
) -> Result<AcpSnapshot, IpcError> {
    acp.transcript(session, since)
        .ok_or_else(|| acp_err(AcpError::NotOpen))
}

/// Start ACP agent session `session_id`'s agent (or keep the running one)
/// and return its state. Loads the session's last ACP session when the
/// agent supports it.
pub async fn acp_open_session(
    ctx: &RpcContext,
    session_id: AgentSessionId,
) -> Result<AcpSnapshot, IpcError> {
    let svc = &ctx.services;
    if svc.acp.is_open(&session_id) {
        return snapshot(&svc.acp, &session_id, 0);
    }
    let session = svc
        .agent_session_store
        .get(&session_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(IpcError::not_found)?;
    if !session.is_open() {
        return Err(IpcError::invalid(format!(
            "agent session {session_id} is closed"
        )));
    }
    let (name, resume) = match (session.kind, session.acp_agent.clone()) {
        (SessionKind::Chat, Some(name)) => (name, session.resume_session_id.clone()),
        _ => return Err(IpcError::invalid("this session doesn't run an ACP agent")),
    };
    let thread = svc
        .thread_store
        .get(&session.thread_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(IpcError::not_found)?;
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
    let agent = agents::find(&svc.acp_adapters, &config, &name)
        .ok_or_else(|| IpcError::invalid(format!("no ACP agent named '{name}' is configured")))?;
    // Checked where it runs: script args are read from the stream's worktree.
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    if !agents::may_start(&svc.approvals, &project_dir, &cwd, &agent) {
        return Err(IpcError::invalid(format!(
            "ACP agent '{name}' is a project program; approve it in Settings → Data → Programs first"
        )));
    }
    let program =
        agents::resolve_command(&project_dir, &agent.program.command).ok_or_else(|| {
            IpcError::invalid(format!(
                "'{}' isn't installed (ACP agent '{name}')",
                agent.program.command
            ))
        })?;

    // The session's bearer is the whole of who its MCP calls come from.
    let endpoints = crate::commands::terminal::session_endpoints(ctx, &session, &thread)?;
    let mcp = vec![McpHttp {
        name: "oxplow".into(),
        url: endpoints.mcp_endpoint_url.clone(),
        headers: vec![(
            "Authorization".into(),
            format!("Bearer {}", endpoints.hook_token),
        )],
    }];
    let harness = svc
        .harnesses
        .get(session.harness.as_str())
        .map_err(|e| IpcError::invalid(e.to_string()))?;
    let system_prompt = oxplow_app::agent_prompt::assemble_acp_system_prompt(
        &project_dir,
        harness.instruction_files(),
        &config,
        &stream,
        Some(&thread),
        &oxplow_app::capabilities::agent_text(ctx),
    );
    // The ACP agent's program, as this project resolves it: the harness's
    // launch makes it the session's process.
    let program = serde_json::json!({
        "program": program,
        "args": agent.program.args,
        "env": agent.program.env.clone().into_iter().collect::<Vec<_>>(),
        "systemPromptViaMeta": agent.system_prompt == SystemPromptVia::Meta,
    });
    let launch = crate::commands::terminal::launch_session(
        ctx,
        harness.as_ref(),
        &endpoints,
        &session,
        &thread,
        &stream,
        None,
        &program,
    )
    .await?;
    let LaunchSpec::Acp {
        program,
        args,
        env,
        system_prompt_via_meta,
    } = launch.spec
    else {
        return Err(IpcError::invalid("this session doesn't run an ACP agent"));
    };
    let spec = SessionSpec {
        session_id,
        thread_id: thread.id,
        agent: name,
        cwd: std::path::PathBuf::from(&stream.worktree_path),
        mcp,
        resume_session_id: Some(resume).filter(|s| !s.is_empty()),
        system_prompt: Some(system_prompt),
        system_prompt_via_meta,
    };
    let launch = Launch {
        program,
        args,
        env,
        env_remove: oxplow_app::agent_path::not_inherited(&svc.harnesses),
    };
    let host = Arc::new(ServicesAcpHost::new(svc, Some(stream.id), session_id));
    svc.acp.open(host, spec, launch).await.map_err(acp_err)?;
    snapshot(&svc.acp, &session_id, 0)
}

/// What the person typed in the prompt box. The one path by which a
/// prompt reaches an ACP agent.
pub async fn acp_prompt(
    svc: &Services,
    session_id: AgentSessionId,
    text: String,
) -> Result<(), IpcError> {
    svc.acp
        .submit_human_prompt(&session_id, text)
        .await
        .map_err(acp_err)
}

/// Stop the running turn (open permission cards are cancelled).
pub async fn acp_cancel(svc: &Services, session_id: AgentSessionId) -> Result<(), IpcError> {
    svc.acp.cancel(&session_id).map_err(acp_err)
}

/// Answer a permission card; no `option_id` cancels it.
pub async fn acp_respond_permission(
    svc: &Services,
    session_id: AgentSessionId,
    request_id: String,
    option_id: Option<String>,
) -> Result<(), IpcError> {
    svc.acp
        .respond_permission(&session_id, request_id, option_id)
        .await
        .map_err(acp_err)
}

/// The session's state and the transcript items changed after `since_seq`
/// (0 for all). `None` when its agent hasn't run in this daemon.
pub async fn acp_transcript(
    svc: &Services,
    session_id: AgentSessionId,
    since_seq: u64,
) -> Result<Option<AcpSnapshot>, IpcError> {
    Ok(svc.acp.transcript(&session_id, since_seq))
}

/// Stop the session's agent process (the session itself stays open: its
/// slot closes with `oxplow.agent_session.close`).
pub async fn acp_close_session(svc: &Services, session_id: AgentSessionId) -> Result<(), IpcError> {
    svc.acp.close(&session_id).map_err(acp_err)
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
            json!({"sessionId": "ses1", "text": "hi"}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("no ACP agent"), "{}", err.message);
        let t = crate::dispatch(
            "acp_transcript",
            json!({"sessionId": "ses1", "sinceSeq": 0}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(t, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn opening_a_terminal_session_as_acp_is_refused() {
        let (ctx, _dir) = crate::test_support::services();
        let stream = ctx.streams.ensure_primary().await.unwrap();
        let thread = ctx
            .threads
            .selected_or_active(&stream.id)
            .await
            .unwrap()
            .unwrap();
        let opened = crate::dispatch(
            "run_command",
            json!({"id": "oxplow.agent_session.open", "input": {
                "thread": oxplow_domain::refs::build::thread_ref(thread),
                "harness": "claude",
            }, "confirmed": false}),
            &ctx,
        )
        .await
        .unwrap();
        let session = opened["result"]["id"].as_str().unwrap().to_string();
        let err = crate::dispatch("acp_open_session", json!({"sessionId": session}), &ctx)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("doesn't run an ACP agent"),
            "{}",
            err.message
        );
    }
}
