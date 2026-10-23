//! A stream's version-control reads, neutral (P5.B4, `.context/vcs.md`):
//! the head, status, blame, a revision's detail and a merge base — each
//! over the `Vcs` capability, naming versions as `Revision`s.

use oxplow_app::vcs::reads;
use oxplow_app::Services;
use oxplow_domain::vcs::{BlameLine, Branch, Revision, RevisionInfo};

use crate::error::IpcError;

pub async fn vcs_blame(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    revision: Revision,
) -> Result<Vec<BlameLine>, IpcError> {
    Ok(reads::blame(svc, stream_id.as_deref(), &path, &revision).await?)
}

pub async fn vcs_merge_base(
    svc: &Services,
    stream_id: Option<String>,
    a: Revision,
    b: Revision,
) -> Result<Option<Revision>, IpcError> {
    Ok(reads::merge_base(svc, stream_id.as_deref(), &a, &b).await?)
}

pub async fn vcs_log(
    svc: &Services,
    stream_id: Option<String>,
    limit: Option<u32>,
    all: bool,
) -> Result<Vec<RevisionInfo>, IpcError> {
    let query = oxplow_domain::vcs::LogQuery {
        limit,
        all,
        ..Default::default()
    };
    Ok(reads::log(svc, stream_id.as_deref(), query).await?)
}

pub async fn vcs_branches(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<Branch>, IpcError> {
    Ok(reads::branches(svc, stream_id.as_deref()).await?)
}

pub async fn vcs_file_history(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    limit: u32,
) -> Result<Vec<RevisionInfo>, IpcError> {
    Ok(reads::file_history(svc, stream_id.as_deref(), &path, limit).await?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// P5.B4 (tsk523): the neutral reads dispatch with revision strings.
    #[tokio::test]
    async fn the_vcs_reads_dispatch() {
        let (svc, dir) = crate::test_support::services().await;
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let sha = git(&["rev-parse", "HEAD"]);
        let call = |name: &'static str, args: serde_json::Value| {
            let svc = svc.clone();
            async move { crate::dispatch(name, args, &svc).await.unwrap() }
        };
        let blame = call(
            "vcs_blame",
            json!({ "streamId": null, "path": "a.txt", "revision": "working" }),
        )
        .await;
        assert_eq!(blame[0]["revision"], json!(sha));
        let base = call(
            "vcs_merge_base",
            json!({ "streamId": null, "a": "git:HEAD", "b": format!("git:{sha}") }),
        )
        .await;
        assert_eq!(base, json!(format!("git:{sha}")));
        let log = call(
            "vcs_log",
            json!({ "streamId": null, "limit": 5, "all": false }),
        )
        .await;
        assert_eq!(log[0]["subject"], json!("c1"));
        let branches = call("vcs_branches", json!({ "streamId": null })).await;
        assert!(
            branches
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["is_default"] == json!(true)),
            "{branches}"
        );
        let history = call(
            "vcs_file_history",
            json!({ "streamId": null, "path": "a.txt", "limit": 10 }),
        )
        .await;
        assert_eq!(history[0]["id"], json!(sha));
    }
}
