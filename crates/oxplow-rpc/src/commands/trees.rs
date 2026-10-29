//! Reading and diffing any version of a stream's tree (P5.B2,
//! `.context/vcs.md` "Trees"): the working tree, a local-history
//! snapshot, or a VCS revision, named by a `Revision` string (`working`,
//! `snap:<id>`, `git:<rev>`). The UI's diff, file and history views read
//! here; none of them knows where the bytes live. A file read names its
//! filesystem (`.context/filesystems.md`); the revision is that
//! filesystem's own word.

use oxplow_app::Services;
use oxplow_domain::filesystem::{FsCx, FsStat};
use oxplow_domain::vcs::Revision;
use oxplow_domain::StreamId;

use crate::error::IpcError;

/// The stream's files on `filesystem` (none: `oxplow:local`), as the
/// registry resolves them.
fn cx(stream_id: Option<&str>) -> FsCx {
    FsCx {
        stream: stream_id.and_then(StreamId::try_from_str),
    }
}

/// `path` at `rev` on `filesystem` (none: the local one) for `stream_id`,
/// as text; `None` when it isn't there. `rev` is the filesystem's own word
/// (the local one's: `working`, `snap:<id>`, `git:<rev>`; none: the working
/// tree).
pub async fn read_at(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    filesystem: Option<String>,
    rev: Option<String>,
) -> Result<Option<String>, IpcError> {
    let fs = svc.filesystems.resolve(filesystem.as_deref())?;
    Ok(fs
        .read(&cx(stream_id.as_deref()), rev.as_deref(), &path, None)
        .await?
        .map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// What `path` is at `rev` on `filesystem`: its kind, size, whether it is
/// text, its language; `None` when it isn't there.
pub async fn stat_file(
    svc: &Services,
    stream_id: Option<String>,
    path: String,
    filesystem: Option<String>,
    rev: Option<String>,
) -> Result<Option<FsStat>, IpcError> {
    let fs = svc.filesystems.resolve(filesystem.as_deref())?;
    Ok(fs
        .stat(&cx(stream_id.as_deref()), rev.as_deref(), &path)
        .await?)
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

    /// The neutral reads take a filesystem and its revision — the local
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
        let read = |rev: Option<String>| {
            call(
                "read_at",
                json!({ "streamId": null, "path": "a.txt", "filesystem": null, "rev": rev }),
            )
        };
        assert_eq!(read(None).await.unwrap(), json!("two\n"));
        assert_eq!(read(Some("working".into())).await.unwrap(), json!("two\n"));
        assert_eq!(
            read(Some(format!("git:{sha}"))).await.unwrap(),
            json!("one\n")
        );
        assert!(read(Some("HEAD".into())).await.is_err(), "not a revision");
        let on = |filesystem: &str| {
            call(
                "read_at",
                json!({ "streamId": null, "path": "a.txt", "filesystem": filesystem, "rev": null }),
            )
        };
        assert_eq!(on("oxplow:local").await.unwrap(), json!("two\n"));
        let e = on("acme:s3").await.unwrap_err();
        assert!(format!("{e:?}").contains("oxplow:local"), "{e:?}");
        let stat = call(
            "stat_file",
            json!({ "streamId": null, "path": "a.txt", "filesystem": null, "rev": null }),
        )
        .await
        .unwrap();
        assert_eq!(stat["kind"], "file");
        assert_eq!(stat["size"], 4);
        assert_eq!(stat["isText"], true);
        assert_eq!(
            call(
                "stat_file",
                json!({ "streamId": null, "path": "nope.txt", "filesystem": null, "rev": null }),
            )
            .await
            .unwrap(),
            json!(null)
        );
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
    }
}
