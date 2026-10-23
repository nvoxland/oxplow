//! Reading and diffing any version of a stream's tree (P5.B2,
//! `.context/vcs.md` "Trees"): the working tree, a local-history
//! snapshot, or a VCS revision, named by a `Revision` string (`working`,
//! `snap:<id>`, `git:<rev>`). The UI's diff, file and history views read
//! here; none of them knows where the bytes live.

use oxplow_app::trees::ChangedPath;
use oxplow_app::Services;
use oxplow_domain::vcs::Revision;

use crate::error::IpcError;

/// `path` at `revision` in `stream_id`'s workspace, as text; `None` when
/// it isn't there.
pub async fn read_at(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    revision: Revision,
) -> Result<Option<String>, IpcError> {
    let ws = svc
        .worktrees
        .resolve(stream_id.as_deref())
        .await
        .into_local_path();
    Ok(svc
        .trees
        .read_at(&ws, &revision, &path)
        .await?
        .map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// Every file at `revision`, sorted.
pub async fn files_at(
    svc: &Services,
    stream_id: Option<String>,
    revision: Revision,
) -> Result<Vec<String>, IpcError> {
    let ws = svc
        .worktrees
        .resolve(stream_id.as_deref())
        .await
        .into_local_path();
    Ok(svc.trees.files_at(&ws, &revision).await?)
}

/// Which paths changed from `from` (nothing, when absent) to `to`, and
/// how — no line counts, so no file is read: what the Project panel's
/// branch and unpushed scopes filter by.
pub async fn changed_paths(
    svc: &Services,
    stream_id: Option<String>,
    from: Option<Revision>,
    to: Revision,
) -> Result<Vec<ChangedPath>, IpcError> {
    let ws = svc
        .worktrees
        .resolve(stream_id.as_deref())
        .await
        .into_local_path();
    Ok(svc.trees.changes(&ws, from.as_ref(), &to).await?)
}

/// Where the `symbol:` ref `symbol` points in `stream_id`: the current symbol, or —
/// for a ref pinned `@snap:`/`@git:` — where it was in that version
/// (`oxplow_app::symbol_resolve`). `None` when it's a dead link.
pub async fn resolve_symbol(
    svc: &Services,
    stream_id: String,
    symbol: String,
) -> Result<Option<oxplow_app::symbol_resolve::SymbolLocation>, IpcError> {
    let stream = oxplow_domain::StreamId::try_from_str(&stream_id).ok_or_else(|| {
        IpcError::from(oxplow_domain::DomainError::Invalid(format!(
            "`{stream_id}` isn't a stream id"
        )))
    })?;
    let ws = svc
        .worktrees
        .resolve(Some(&stream_id))
        .await
        .into_local_path();
    Ok(oxplow_app::symbol_resolve::resolve(svc, stream, &ws, &symbol).await?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// P5.B2 (tsk521): the neutral reads take a revision string — the
    /// working tree and a commit here — and a malformed one is refused.
    #[tokio::test]
    async fn reads_and_changed_paths_take_a_revision() {
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
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        let call = |name: &'static str, args: serde_json::Value| {
            let svc = svc.clone();
            async move { crate::dispatch(name, args, &svc).await }
        };
        let read = |rev: String| {
            call(
                "read_at",
                json!({ "streamId": null, "path": "a.txt", "revision": rev }),
            )
        };
        assert_eq!(read("working".into()).await.unwrap(), json!("two\n"));
        assert_eq!(read(format!("git:{sha}")).await.unwrap(), json!("one\n"));
        assert!(read("HEAD".into()).await.is_err(), "not a revision");
        let files = call(
            "files_at",
            json!({ "streamId": null, "revision": format!("git:{sha}") }),
        )
        .await
        .unwrap();
        assert!(
            files.as_array().unwrap().contains(&json!("a.txt")),
            "{files}"
        );
        let paths = call(
            "changed_paths",
            json!({ "streamId": null, "from": format!("git:{sha}"), "to": "working" }),
        )
        .await
        .unwrap();
        assert_eq!(paths, json!([{ "path": "a.txt", "status": "modified" }]));
    }
}
