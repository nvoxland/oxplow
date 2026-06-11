use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_app::lsp_installer::LspInstallerError;
use oxplow_app::terminal_sessions::TerminalSessionError;
use oxplow_app::TaskServiceError;
use oxplow_domain::DomainError;
use oxplow_session::{SessionError, ThreadError};

/// Frontend-facing error envelope.
///
/// All command cores return `Result<T, IpcError>`, and the Tauri
/// `#[tauri::command]` wrappers + the daemon's HTTP routes surface this
/// same shape. Internal errors from the service layer are converted here
/// so the JS side never has to reason about Rust-specific error types.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct IpcError {
    pub code: String,
    pub message: String,
    pub cause: Option<String>,
}

impl IpcError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL".into(),
            message: msg.into(),
            cause: None,
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self {
            code: "INVALID".into(),
            message: msg.into(),
            cause: None,
        }
    }

    pub fn not_found() -> Self {
        Self {
            code: "NOT_FOUND".into(),
            message: "not found".into(),
            cause: None,
        }
    }

    pub fn with_cause(mut self, cause: impl ToString) -> Self {
        self.cause = Some(cause.to_string());
        self
    }
}

impl From<DomainError> for IpcError {
    fn from(value: DomainError) -> Self {
        match &value {
            DomainError::Invalid(msg) => Self {
                code: "INVALID".into(),
                message: msg.clone(),
                cause: None,
            },
            DomainError::NotFound => Self::not_found(),
            DomainError::Invariant(msg) => Self {
                code: "INVARIANT".into(),
                message: msg.clone(),
                cause: None,
            },
            DomainError::Constraint(msg) => Self {
                code: "CONSTRAINT".into(),
                message: msg.clone(),
                cause: None,
            },
            DomainError::Busy(msg) => Self {
                code: "BUSY".into(),
                message: msg.clone(),
                cause: None,
            },
            DomainError::Storage(msg) => Self {
                code: "STORAGE".into(),
                message: msg.clone(),
                cause: None,
            },
        }
    }
}

impl From<SessionError> for IpcError {
    fn from(value: SessionError) -> Self {
        match &value {
            SessionError::NotARepo(p) => Self {
                code: "NOT_A_REPO".into(),
                message: format!("not a git repo: {}", p.display()),
                cause: None,
            },
            SessionError::InWorktree(p) => Self {
                code: "IN_WORKTREE".into(),
                message: format!("workspace is a secondary git worktree: {}", p.display()),
                cause: None,
            },
            SessionError::PrimaryExists => Self {
                code: "PRIMARY_EXISTS".into(),
                message: "primary stream already exists".into(),
                cause: None,
            },
            SessionError::PrimaryMissing => Self {
                code: "PRIMARY_MISSING".into(),
                message: "primary stream missing".into(),
                cause: None,
            },
            SessionError::DuplicateWorktreeSlug(slug) => Self {
                code: "DUPLICATE_WORKTREE_SLUG".into(),
                message: format!("worktree slug \"{slug}\" already exists"),
                cause: None,
            },
            SessionError::Git(e) => Self {
                code: "GIT".into(),
                message: e.to_string(),
                cause: None,
            },
            SessionError::Storage(e) => IpcError::from(e.clone()),
        }
    }
}

impl From<TaskServiceError> for IpcError {
    fn from(value: TaskServiceError) -> Self {
        match value {
            TaskServiceError::NotFound(_) => IpcError::not_found(),
            TaskServiceError::Storage(e) => IpcError::from(e),
        }
    }
}

impl From<ThreadError> for IpcError {
    fn from(value: ThreadError) -> Self {
        match value {
            ThreadError::NotFound(_) => IpcError::not_found(),
            ThreadError::Closed(id) => Self {
                code: "THREAD_CLOSED".into(),
                message: format!("thread is closed: {id}"),
                cause: None,
            },
            ThreadError::Storage(e) => IpcError::from(e),
        }
    }
}

impl From<TerminalSessionError> for IpcError {
    fn from(value: TerminalSessionError) -> Self {
        match value {
            TerminalSessionError::NotFound(id) => {
                IpcError::invalid(format!("terminal session not found: {id}"))
            }
            TerminalSessionError::Pty(e) => IpcError::internal(e.to_string()),
            TerminalSessionError::InvalidMessage(msg) => IpcError::invalid(msg),
            TerminalSessionError::Base64(msg) => IpcError::invalid(format!("base64: {msg}")),
        }
    }
}

impl From<LspInstallerError> for IpcError {
    fn from(value: LspInstallerError) -> Self {
        IpcError::internal(value.to_string())
    }
}

impl From<oxplow_app::lsp_sessions::LspSessionError> for IpcError {
    fn from(value: oxplow_app::lsp_sessions::LspSessionError) -> Self {
        use oxplow_app::lsp_sessions::LspSessionError;
        match value {
            // NoConfig carries its own self-describing, actionable
            // message (suggested Mason package + fix paths).
            e @ LspSessionError::NoConfig(_) => IpcError::invalid(e.to_string()),
            e => IpcError::internal(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_sets_internal_code() {
        let e = IpcError::internal("boom");
        assert_eq!(e.code, "INTERNAL");
        assert_eq!(e.message, "boom");
        assert!(e.cause.is_none());
    }

    #[test]
    fn invalid_sets_invalid_code() {
        let e = IpcError::invalid("bad input");
        assert_eq!(e.code, "INVALID");
        assert_eq!(e.message, "bad input");
    }

    #[test]
    fn not_found_factory() {
        let e = IpcError::not_found();
        assert_eq!(e.code, "NOT_FOUND");
        assert_eq!(e.message, "not found");
    }

    #[test]
    fn with_cause_attaches_string() {
        let inner = std::io::Error::other("io fault");
        let e = IpcError::internal("wrapped").with_cause(inner);
        assert_eq!(e.cause.as_deref(), Some("io fault"));
    }

    #[test]
    fn from_domain_invalid_uses_invalid_code() {
        let e: IpcError = DomainError::Invalid("bad".into()).into();
        assert_eq!(e.code, "INVALID");
        assert_eq!(e.message, "bad");
    }

    #[test]
    fn from_domain_not_found_maps_to_not_found() {
        let e: IpcError = DomainError::NotFound.into();
        assert_eq!(e.code, "NOT_FOUND");
    }

    #[test]
    fn from_domain_invariant_uses_invariant_code() {
        let e: IpcError = DomainError::Invariant("rule".into()).into();
        assert_eq!(e.code, "INVARIANT");
    }

    #[test]
    fn from_domain_storage_variants_use_dedicated_codes() {
        let e: IpcError = DomainError::Constraint("dup".into()).into();
        assert_eq!(e.code, "CONSTRAINT");
        let e: IpcError = DomainError::Busy("locked".into()).into();
        assert_eq!(e.code, "BUSY");
        let e: IpcError = DomainError::Storage("io".into()).into();
        assert_eq!(e.code, "STORAGE");
    }

    #[test]
    fn from_session_not_a_repo_maps() {
        let e: IpcError = SessionError::NotARepo("/no/such".into()).into();
        assert_eq!(e.code, "NOT_A_REPO");
        assert!(e.message.contains("/no/such"));
    }

    #[test]
    fn from_session_primary_missing_maps() {
        let e: IpcError = SessionError::PrimaryMissing.into();
        assert_eq!(e.code, "PRIMARY_MISSING");
    }

    #[test]
    fn ipc_error_serde_round_trips() {
        let e = IpcError::internal("hi").with_cause("inner");
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, "INTERNAL");
        assert_eq!(back.message, "hi");
        assert_eq!(back.cause.as_deref(), Some("inner"));
    }

    #[test]
    fn from_session_in_worktree_maps_with_path_in_message() {
        let e: IpcError =
            SessionError::InWorktree(std::path::PathBuf::from("/wt/secondary")).into();
        assert_eq!(e.code, "IN_WORKTREE");
        assert!(e.message.contains("/wt/secondary"), "msg: {}", e.message);
    }

    #[test]
    fn from_session_primary_exists_maps() {
        let e: IpcError = SessionError::PrimaryExists.into();
        assert_eq!(e.code, "PRIMARY_EXISTS");
    }

    #[test]
    fn from_session_duplicate_slug_includes_slug() {
        let e: IpcError = SessionError::DuplicateWorktreeSlug("feature".into()).into();
        assert_eq!(e.code, "DUPLICATE_WORKTREE_SLUG");
        assert!(e.message.contains("feature"), "msg: {}", e.message);
    }

    #[test]
    fn from_session_storage_passes_through_domain_mapping() {
        // Storage(NotFound) should land as NOT_FOUND, not GIT.
        let e: IpcError = SessionError::Storage(DomainError::NotFound).into();
        assert_eq!(e.code, "NOT_FOUND");
    }

    #[test]
    fn from_task_service_not_found_maps_to_not_found() {
        let e: IpcError = TaskServiceError::NotFound(oxplow_domain::TaskId::new(7)).into();
        assert_eq!(e.code, "NOT_FOUND");
    }

    #[test]
    fn from_task_service_storage_propagates() {
        let e: IpcError = TaskServiceError::Storage(DomainError::Invalid("bad row".into())).into();
        assert_eq!(e.code, "INVALID");
        assert_eq!(e.message, "bad row");
    }

    #[test]
    fn from_thread_error_not_found_maps_to_not_found() {
        let e: IpcError = ThreadError::NotFound(oxplow_domain::ThreadId::new(1)).into();
        assert_eq!(e.code, "NOT_FOUND");
    }

    #[test]
    fn from_thread_error_closed_uses_dedicated_code() {
        let id = oxplow_domain::ThreadId::new(2);
        let e: IpcError = ThreadError::Closed(id).into();
        assert_eq!(e.code, "THREAD_CLOSED");
        assert!(e.message.contains(&id.to_string()), "msg: {}", e.message);
    }

    #[test]
    fn from_thread_error_storage_passes_through() {
        let e: IpcError = ThreadError::Storage(DomainError::Invariant("oops".into())).into();
        assert_eq!(e.code, "INVARIANT");
        assert_eq!(e.message, "oops");
    }

    #[test]
    fn ipc_error_display_uses_message() {
        // The Error/Display impl drives logging — make sure the
        // human-readable message round-trips, not Debug-formatted noise.
        let e = IpcError::invalid("user-friendly message");
        assert_eq!(format!("{e}"), "user-friendly message");
    }
}
