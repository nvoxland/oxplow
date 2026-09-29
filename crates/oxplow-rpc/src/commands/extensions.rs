//! Cores for the `extensions` command module — project extensions and
//! their lenses, read from a stream's worktree. See
//! `.context/extensions.md`.

use std::collections::BTreeMap;

use oxplow_app::extensions::{self, Extension, Lens, LensRun, NewLens};
use oxplow_app::Services;
use oxplow_db::SqlCell;
use oxplow_domain::DomainError;

use crate::error::IpcError;

/// The worktree whose `oxplow/extensions/` a call reads: the stream's,
/// or the primary's when `stream_id` is omitted.
async fn root(svc: &Services, stream_id: Option<&str>) -> std::path::PathBuf {
    svc.git.resolve_repo_dir(stream_id).await
}

/// Every project extension in the stream's worktree (primary when
/// `stream_id` is omitted), with per-extension load errors.
pub async fn list_extensions(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<Extension>, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(svc.extension_catalog.get(&root).to_vec())
}

/// One lens by `<extension>/<slug>`.
pub async fn get_lens(
    svc: &Services,
    id: String,
    stream_id: Option<String>,
) -> Result<Lens, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(svc.extension_catalog.find_lens(&root, &id)?)
}

/// Run a lens with optional param overrides; returns the rows the lens
/// page shows.
pub async fn run_lens(
    svc: &Services,
    id: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<LensRun, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        &root,
        &id,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?)
}

/// The viewer's context: the stream (primary when omitted) and its
/// selected thread, bound into lenses that declare `stream_id` /
/// `thread_id`.
async fn context(svc: &Services, stream_id: Option<&str>) -> extensions::LensContext {
    let stream = stream_id.and_then(oxplow_domain::StreamId::try_from_str);
    extensions::lens_context(svc, stream, None).await
}

/// Run one of a lens's declared actions. It never approves an exec
/// source: that consent is given in Settings → Data, where what runs and
/// which hosts it reaches are shown.
pub async fn run_lens_action(
    svc: &Services,
    id: String,
    action: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<oxplow_app::lens_actions::LensActionResult, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::run_lens_action(
        svc,
        &root,
        &id,
        &action,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?)
}

/// Load one extension and dry-run each lens, returning every problem —
/// the SDK's `check`, the same report `oxplow plugin check` prints.
pub async fn validate_extension(
    svc: &Services,
    name: String,
    stream_id: Option<String>,
) -> Result<oxplow_sdk::CheckReport, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    oxplow_sdk::check(&root, &name, &svc.extension_catalog, Some(&svc.sql))
        .await
        .map_err(sdk_error)
}

fn sdk_error(e: oxplow_sdk::SdkError) -> IpcError {
    match e {
        oxplow_sdk::SdkError::NotFound(_) => DomainError::NotFound.into(),
        oxplow_sdk::SdkError::Domain(d) => d.into(),
        other => DomainError::Invalid(other.to_string()).into(),
    }
}

/// Turn an extension on or off for the project (`extensions.disabled` in
/// `.oxplow/project.yaml`, so it's team-wide once committed). Returns the
/// primary stream's extensions. UI only: agents don't toggle extensions.
pub async fn set_extension_enabled(
    svc: &Services,
    name: String,
    enabled: bool,
) -> Result<Vec<Extension>, IpcError> {
    oxplow_app::config_service::mutate_config(&svc.config, &svc.layout.project_dir, |c| {
        c.extensions_disabled.retain(|d| d != &name);
        if !enabled {
            c.extensions_disabled.push(name.clone());
        }
    })
    .map_err(|e| IpcError::invalid(e.to_string()))?;
    svc.events
        .emit(oxplow_app::events::OxplowEvent::ConfigChanged);
    let root = root(svc, None).await;
    Ok(svc.extension_catalog.get(&root).to_vec())
}

/// What installing (`git_url`) or updating (`name`) an extension would
/// bring in, installing nothing: shown for the person to confirm.
pub async fn review_extension(
    svc: &Services,
    git_url: Option<String>,
    git_ref: Option<String>,
    name: Option<String>,
    stream_id: Option<String>,
) -> Result<extensions::ExtensionReview, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(match (git_url, name) {
        (Some(url), None) => {
            extensions::review_extension(
                &svc.sql,
                &svc.extension_catalog,
                &root,
                &url,
                git_ref.as_deref(),
                None,
            )
            .await?
        }
        (None, Some(name)) => {
            extensions::review_update(&svc.sql, &svc.extension_catalog, &root, &name).await?
        }
        _ => {
            return Err(IpcError::invalid(
                "pass either gitUrl (install) or name (update)",
            ))
        }
    })
}

/// Install an extension from a git repo into this stream's worktree
/// (`oxplow/extensions/<name>/`), at the commit the person reviewed. The
/// files are then ordinary project files: commit them to share with the
/// team.
pub async fn install_extension(
    svc: &Services,
    git_url: String,
    git_ref: Option<String>,
    reviewed_sha: String,
    stream_id: Option<String>,
) -> Result<Extension, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    // git clone + file copy: blocking work off the async runtime.
    let ext = tokio::task::spawn_blocking(move || {
        extensions::install_extension(&root, &git_url, git_ref.as_deref(), &reviewed_sha)
    })
    .await
    .map_err(|e| IpcError::internal(format!("install task panicked: {e}")))??;
    Ok(ext)
}

/// Re-install an installed extension from its recorded git source.
pub async fn update_extension(
    svc: &Services,
    name: String,
    reviewed_sha: String,
    stream_id: Option<String>,
) -> Result<Extension, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ext = tokio::task::spawn_blocking(move || {
        extensions::update_extension(&root, &name, &reviewed_sha)
    })
    .await
    .map_err(|e| IpcError::internal(format!("update task panicked: {e}")))??;
    Ok(ext)
}

/// Save a query from Explore Data as a new lens file in this stream's
/// worktree. UI-only: agents write lens files with their Edit tool.
pub async fn save_lens(
    svc: &Services,
    extension: String,
    slug: String,
    lens: NewLens,
    stream_id: Option<String>,
) -> Result<Lens, IpcError> {
    // A lens reads published models only: the explorer's raw mode can
    // run a physical-table query, but it can't be saved as one.
    svc.sql.check(&lens.query).await?;
    let root = root(svc, stream_id.as_deref()).await;
    Ok(extensions::save_lens(&root, &extension, &slug, lens)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn write(root: &std::path::Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn seed(root: &std::path::Path) {
        write(
            root,
            "oxplow/extensions/demo/extension.yaml",
            "name: demo\ndescription: Demo\n",
        );
        write(
            root,
            "oxplow/extensions/demo/lenses/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        );
    }

    #[tokio::test]
    async fn lists_gets_runs_and_validates_lenses() {
        let (svc, _dir) = crate::test_support::services();
        seed(&svc.layout.project_dir);

        let exts = crate::dispatch("list_extensions", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(exts[0]["name"], "demo");
        assert_eq!(exts[0]["lenses"][0]["id"], "demo/streams");

        let lens = crate::dispatch("get_lens", json!({ "id": "demo/streams" }), &svc)
            .await
            .unwrap();
        assert_eq!(lens["title"], "Streams");

        let run = crate::dispatch("run_lens", json!({ "id": "demo/streams" }), &svc)
            .await
            .unwrap();
        assert_eq!(run["result"]["rows"], json!([["primary"]]));
        let run = crate::dispatch(
            "run_lens",
            json!({ "id": "demo/streams", "params": { "kind": "worktree" } }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(run["result"]["rows"], json!([]));

        let v = crate::dispatch("validate_extension", json!({ "name": "demo" }), &svc)
            .await
            .unwrap();
        assert_eq!(v["errors"], json!([]));
    }

    #[tokio::test]
    async fn missing_lens_is_not_found_and_bad_params_are_invalid() {
        let (svc, _dir) = crate::test_support::services();
        seed(&svc.layout.project_dir);
        let err = crate::dispatch("get_lens", json!({ "id": "demo/nope" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
        let err = crate::dispatch(
            "run_lens",
            json!({ "id": "demo/streams", "params": { "knd": "x" } }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        assert!(std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
    }

    #[tokio::test]
    async fn installs_and_updates_from_git() {
        let (svc, _dir) = crate::test_support::services();
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), "extension.yaml", "name: shared\n");
        write(
            repo.path(),
            "lenses/one.yaml",
            "title: One\nquery: SELECT 1\n",
        );
        git(repo.path(), &["init", "-q", "-b", "main"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let url = repo.path().to_string_lossy().to_string();

        let review = crate::dispatch("review_extension", json!({ "gitUrl": url }), &svc)
            .await
            .unwrap();
        assert_eq!(review["extension"]["name"], "shared");
        let sha = review["sha"].clone();
        let ext = crate::dispatch(
            "install_extension",
            json!({ "gitUrl": url, "reviewedSha": sha }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(ext["name"], "shared");
        assert_eq!(ext["source"]["git"], json!(url));

        let err = crate::dispatch(
            "install_extension",
            json!({ "gitUrl": url, "reviewedSha": sha }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");

        write(
            repo.path(),
            "lenses/one.yaml",
            "title: One v2\nquery: SELECT 1\n",
        );
        git(repo.path(), &["commit", "-q", "-am", "v2"]);
        let review = crate::dispatch("review_extension", json!({ "name": "shared" }), &svc)
            .await
            .unwrap();
        let ext = crate::dispatch(
            "update_extension",
            json!({ "name": "shared", "reviewedSha": review["sha"] }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(ext["lenses"][0]["title"], "One v2");
    }

    #[tokio::test]
    async fn save_lens_writes_a_runnable_lens() {
        let (svc, _dir) = crate::test_support::services();
        let lens = crate::dispatch(
            "save_lens",
            json!({
                "extension": "mine",
                "slug": "streams",
                "lens": { "title": "Streams", "query": "SELECT kind FROM v_stream", "viz": "table" }
            }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(lens["id"], "mine/streams");
        let run = crate::dispatch("run_lens", json!({ "id": "mine/streams" }), &svc)
            .await
            .unwrap();
        assert_eq!(run["result"]["rows"], json!([["primary"]]));
        // A query over a physical table (the explorer's raw mode) can't be
        // saved as a lens.
        let err = crate::dispatch(
            "save_lens",
            json!({
                "extension": "mine",
                "slug": "raw",
                "lens": { "title": "Raw", "query": "SELECT kind FROM streams", "viz": "table" }
            }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert!(
            err.message.contains("`streams` is a physical table"),
            "{}",
            err.message
        );
    }

    #[tokio::test]
    async fn extensions_can_be_disabled_and_reenabled() {
        let (svc, _dir) = crate::test_support::services();
        let enabled = |list: &serde_json::Value| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|e| e["name"] == "oxplow-review")
                .unwrap()["enabled"]
                .clone()
        };
        let list = crate::dispatch(
            "set_extension_enabled",
            json!({ "name": "oxplow-review", "enabled": false }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(enabled(&list), json!(false));
        let yaml =
            std::fs::read_to_string(svc.layout.project_dir.join(".oxplow/project.yaml")).unwrap();
        assert!(yaml.contains("oxplow-review"), "{yaml}");
        let list = crate::dispatch(
            "set_extension_enabled",
            json!({ "name": "oxplow-review", "enabled": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(enabled(&list), json!(true));
    }
}
