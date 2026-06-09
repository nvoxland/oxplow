//! Agent pane lifecycle.
//!
//! Bridges `Stream` + `agent_command` + `oxplow-tmux` so callers
//! can say "make sure stream X has its working/talking panes
//! running" and get a tmux window per pane on demand. Idempotent —
//! existing windows are reused unless their launcher signature
//! changed (signaling the user edited config and we need a fresh
//! launch).

use std::sync::Arc;

use thiserror::Error;
use tracing::info;

use oxplow_domain::{AgentKind, Stream};
use oxplow_tmux::{Session, TmuxError, TmuxRunner, WindowTarget};

use crate::agent_command::{build_agent_command, AgentCommandOptions, PaneKind};

/// Default tmux session name for an oxplow-managed stream. Per-stream
/// so worktrees stay isolated.
pub fn session_for_stream(stream: &Stream) -> Session {
    Session(format!("oxplow-{}", sanitize(&stream.id.to_string())))
}

/// Window names within a session — one per pane kind.
pub fn window_name_for(pane: PaneKind) -> &'static str {
    match pane {
        PaneKind::Working => "working",
        PaneKind::Talking => "talking",
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum AgentPaneError {
    #[error("tmux: {0}")]
    Tmux(#[from] TmuxError),
}

/// Default geometry for fresh panes. Renderer drives explicit
/// resizes via `resize_window` once attached.
pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 36;

#[derive(Clone)]
pub struct AgentPaneService {
    tmux: Arc<dyn TmuxRunner>,
}

impl AgentPaneService {
    pub fn new(tmux: Arc<dyn TmuxRunner>) -> Self {
        Self { tmux }
    }

    /// Compute a stable launcher signature so we can tell whether the
    /// agent command oxplow *would* run today matches the one already
    /// running in the existing window. If they diverge tmux gets a
    /// fresh window; otherwise the existing one is reused.
    fn launcher_signature(command: &str) -> String {
        // Hash-light fingerprint — short enough to fit in a tmux
        // option, deterministic over the input.
        format!("{:x}", djb2(command))
    }

    pub async fn ensure_pane(
        &self,
        stream: &Stream,
        pane: PaneKind,
        agent: AgentKind,
        opts: AgentCommandOptions,
    ) -> Result<EnsurePaneOutcome, AgentPaneError> {
        let session = session_for_stream(stream);
        let cwd = std::path::Path::new(&stream.worktree_path);
        self.tmux.ensure_session(&session, cwd).await?;

        let target = WindowTarget::from_parts(&session, window_name_for(pane));
        let command = build_agent_command(agent, stream, pane, &opts);
        let signature = Self::launcher_signature(&command);

        let created = self
            .tmux
            .ensure_window(
                &target,
                cwd,
                &command,
                DEFAULT_COLS,
                DEFAULT_ROWS,
                Some(&signature),
            )
            .await?;
        info!(
            stream_id = %stream.id,
            pane = ?pane,
            target = target.as_str(),
            created,
            "agent pane ensured"
        );
        Ok(EnsurePaneOutcome {
            session,
            target,
            created,
        })
    }

    /// Tear down all oxplow-managed panes for a stream. Used when the
    /// user deletes a stream / closes a worktree.
    pub async fn teardown_stream(&self, stream: &Stream) {
        let session = session_for_stream(stream);
        self.tmux.kill_session(&session).await;
    }
}

/// Result of `ensure_pane`. `created=true` means tmux just spawned a
/// fresh window; `false` means it was already running and reused.
#[derive(Debug, Clone)]
pub struct EnsurePaneOutcome {
    pub session: Session,
    pub target: WindowTarget,
    pub created: bool,
}

/// Cheap DJB2 string hash. Stable; collisions are tolerable here
/// because the worst case is "treat a re-launch as fresh".
fn djb2(s: &str) -> u64 {
    let mut hash: u64 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, StreamKind, Timestamp};

    fn stream() -> Stream {
        Stream {
            id: StreamId::new(1),
            kind: StreamKind::Worktree,
            title: "feat".into(),
            branch: "feat".into(),
            branch_ref: "refs/heads/feat".into(),
            branch_source: "main".into(),
            worktree_path: "/repo/wt".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    #[test]
    fn session_name_is_sanitized() {
        let s = stream();
        let sess = session_for_stream(&s);
        // Forward slashes from the id get replaced with dashes so
        // tmux session names stay valid.
        assert!(sess.as_str().starts_with("oxplow-"));
        assert!(!sess.as_str().contains('/'));
    }

    #[test]
    fn window_names_are_per_pane_kind() {
        assert_eq!(window_name_for(PaneKind::Working), "working");
        assert_eq!(window_name_for(PaneKind::Talking), "talking");
    }

    #[test]
    fn launcher_signature_is_stable_for_same_command() {
        let a = AgentPaneService::launcher_signature("foo");
        let b = AgentPaneService::launcher_signature("foo");
        let c = AgentPaneService::launcher_signature("bar");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
