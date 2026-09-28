//! Thread lifecycle steps that reach past the thread row, shared by the
//! RPC and MCP surfaces so neither forgets one.

use oxplow_domain::{AgentKind, Thread, ThreadId};
use oxplow_session::ThreadError;

use crate::Services;

/// Close a thread. An ACP thread's agent session (and process) stops with
/// it; a terminal agent's pane is the terminal's to end.
pub async fn close_thread(svc: &Services, id: &ThreadId) -> Result<Thread, ThreadError> {
    let thread = svc.threads.close(id).await?;
    if thread.agent == AgentKind::Acp {
        // Not open is fine: there's nothing to stop.
        let _ = svc.acp.close(id);
    }
    Ok(thread)
}
