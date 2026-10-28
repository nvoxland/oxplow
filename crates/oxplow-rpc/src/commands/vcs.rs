//! The person's version-control reads (`.context/vcs.md`): blame, the
//! agent's read run as the person. History and branches are the agent's
//! `oxplow.vcs.log` / `oxplow.vcs.branches` and the `v_commit` / `v_branch`
//! models.

use oxplow_app::Services;
use oxplow_domain::vcs::{BlameLine, Revision};

use crate::error::IpcError;

/// The blame of `path` at `revision`: the agent's `oxplow.vcs.blame`, run
/// as the person.
pub async fn vcs_blame(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    revision: Revision,
) -> Result<Vec<BlameLine>, IpcError> {
    crate::commands::bus::read_as_person(
        svc,
        "oxplow.vcs.blame",
        serde_json::json!({
            "stream_id": stream_id,
            "path": path,
            "revision": revision.to_string(),
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// The neutral read dispatches with a revision string.
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
    }

    /// The person's blame is the agent's: a stream id that isn't one is
    /// refused at its field.
    #[tokio::test]
    async fn blame_refuses_a_thread_id_as_the_stream() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch(
            "vcs_blame",
            json!({ "streamId": "thr1", "path": "a.txt", "revision": "working" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID", "{err:?}");
        assert_eq!(err.field.as_deref(), Some("/stream_id"), "{err:?}");
    }
}
