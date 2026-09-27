//! Cores for the `extensions` command module — project extensions and
//! their lenses, read from a stream's worktree. See
//! `.context/extensions.md`.

use std::collections::BTreeMap;

use oxplow_app::extensions::{self, Extension, Lens, LensRun};
use oxplow_app::Services;
use oxplow_db::{SemanticLayer, SqlCell};

use crate::error::IpcError;

/// The worktree whose `oxplow/extensions/` a call reads: the stream's,
/// or the primary's when `stream_id` is omitted.
async fn root(svc: &Services, stream_id: Option<&str>) -> std::path::PathBuf {
    svc.git.resolve_repo_dir(stream_id).await
}

fn layer(svc: &Services) -> SemanticLayer {
    SemanticLayer::new(svc.db.clone())
}

/// Every project extension in the stream's worktree (primary when
/// `stream_id` is omitted), with per-extension load errors.
pub async fn list_extensions(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<Extension>, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(extensions::load_extensions(&root))
}

/// One lens by `<extension>/<slug>`.
pub async fn get_lens(
    svc: &Services,
    id: String,
    stream_id: Option<String>,
) -> Result<Lens, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(extensions::find_lens(&root, &id)?)
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
    Ok(extensions::run_lens(&layer(svc), &root, &id, params.unwrap_or_default()).await?)
}

/// Load one extension and dry-run each lens, returning every problem.
pub async fn validate_extension(
    svc: &Services,
    name: String,
    stream_id: Option<String>,
) -> Result<Extension, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(extensions::validate_extension(&layer(svc), &root, &name).await?)
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
}
