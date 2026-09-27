//! Cores for the `changes` command module: analyze a change (a commit,
//! an effort, or the working tree) and return its row. The analysis is
//! read through `v_change*`. See `.context/semantic-layer.md`.

use oxplow_app::change_analysis::{self, ChangeTarget};
use oxplow_app::Services;
use oxplow_db::ChangeRow;

use crate::error::IpcError;

/// Analyze `target` if needed and return the change (`status` is `done`,
/// or `running` while another request computes it).
pub async fn ensure_change(svc: &Services, target: ChangeTarget) -> Result<ChangeRow, IpcError> {
    Ok(change_analysis::ensure_change(svc, target).await?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn a_commit_change_is_analyzed_over_ipc() {
        let (svc, dir) = crate::test_support::services();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let status = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        };
        assert!(status(&["add", "."]) && status(&["commit", "-q", "-m", "a"]));
        let row = crate::dispatch(
            "ensure_change",
            json!({ "target": { "kind": "commit", "sha": "HEAD" } }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(
            (row["kind"].clone(), row["status"].clone()),
            (json!("commit"), json!("done"))
        );
        let q = crate::dispatch(
            "query_sql",
            json!({ "sql": format!(
                "SELECT path, status FROM v_change_file WHERE change_id = {} AND path = 'a.rs'",
                row["id"]
            ) }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(q["rows"], json!([["a.rs", "added"]]));
        let err = crate::dispatch(
            "ensure_change",
            json!({ "target": { "kind": "commit", "sha": "nope" } }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
