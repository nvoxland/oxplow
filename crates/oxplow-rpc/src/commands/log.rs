//! Cores for the `log` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_git::{CommitDetail, GitLogCommit, GitLogOptions, GitLogResult};

use crate::error::IpcError;

pub async fn get_git_log(
    svc: &Services,
    stream_id: Option<String>,
    limit: Option<u32>,
    all: bool,
) -> Result<GitLogResult, IpcError> {
    let opts = GitLogOptions {
        limit: limit.map(|n| n as usize),
        all,
    };
    Ok(svc.git.git_log(stream_id.as_deref(), opts).await)
}

pub async fn get_commit_detail(
    svc: &Services,
    stream_id: Option<String>,
    sha: String,
) -> Result<Option<CommitDetail>, IpcError> {
    Ok(svc.git.commit_detail(stream_id.as_deref(), sha).await)
}

pub async fn get_commits_ahead_of(
    svc: &Services,
    stream_id: Option<String>,
    base: String,
    head: String,
    limit: u32,
) -> Result<Vec<GitLogCommit>, IpcError> {
    Ok(svc
        .git
        .commits_ahead_of(stream_id.as_deref(), base, head, limit as usize)
        .await)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn get_git_log_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("get_git_log", serde_json::json!({ "all": false }), &svc)
            .await
            .unwrap();
        assert!(out.is_object(), "expected a result object, got {out}");
    }

    #[tokio::test]
    async fn get_commit_detail_returns_null_for_missing_sha() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_commit_detail",
            serde_json::json!({ "sha": "0000000000000000000000000000000000000000" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out, serde_json::json!(null));
    }
}
