//! Cores for the `extensions` command module — project extensions and
//! their lenses, read from a stream's worktree. See
//! `.context/extensions.md`.

use std::collections::BTreeMap;

use oxplow_app::extensions::{self, Extension, Lens, LensRun};
use oxplow_app::Services;
use oxplow_db::SqlCell;
use oxplow_domain::DomainError;

use crate::error::IpcError;

/// The worktree whose `oxplow/extensions/` a call reads: the stream's,
/// or the primary's when `stream_id` is omitted.
async fn root(svc: &Services, stream_id: Option<&str>) -> std::path::PathBuf {
    svc.worktrees.resolve(stream_id).await
}

/// Every project extension in the stream's worktree (primary when
/// `stream_id` is omitted), with per-extension load errors.
pub async fn list_extensions(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<Extension>, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    Ok(svc.listed_extensions(&root).await)
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

/// Run an answer shown in a thread (`answer:<id>`) as the person sees it.
pub async fn run_answer(svc: &Services, answer: String) -> Result<LensRun, IpcError> {
    let id: i64 = answer
        .strip_prefix("answer:")
        .unwrap_or(&answer)
        .parse()
        .map_err(|_| IpcError::invalid(format!("`{answer}` isn't an answer (`answer:<id>`)")))?;
    Ok(oxplow_app::commands::lens::run_answer(svc, id).await?)
}

/// A lens's text rendering (`lens_text`): what Copy puts on the
/// clipboard.
pub async fn lens_text(
    svc: &Services,
    id: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<String, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    let run = extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        &root,
        &id,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?;
    Ok(oxplow_app::lens_text::text_of(svc, &root, &run, &ctx).await?)
}

/// What a form lens shows: its command's spec (the fields) and the values
/// they start from.
pub async fn lens_form(
    svc: &Services,
    id: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(
        oxplow_app::lens_actions::form_start(svc, &root, &id, params.unwrap_or_default(), &ctx)
            .await?,
    )
}

/// A person submits a form lens: its command runs as the lens, acting for
/// them (`NEEDS_CONFIRMATION` asks them first).
pub async fn submit_lens_form(
    svc: &Services,
    id: String,
    input: oxplow_domain::Json,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
    confirmed: bool,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::submit_form(
        svc,
        &root,
        oxplow_app::lens_actions::FormSubmission {
            lens_id: id,
            input: input.0,
            params: params.unwrap_or_default(),
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed,
        },
        &ctx,
    )
    .await?)
}

/// A custom component's frame reads one of its declared lenses (P6b.D2):
/// the person is looking at lens `id`; `asset` names the lens to run.
pub async fn run_component_query(
    svc: &Services,
    id: String,
    asset: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<LensRun, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::run_component_query(
        svc,
        &root,
        &id,
        &asset,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?)
}

/// The worktree a component's lens is shown in: the stream's, or the
/// primary's outside any stream — never the primary's for a stream that is
/// gone (tsk984).
async fn component_root(
    svc: &Services,
    stream_id: Option<&str>,
) -> Result<std::path::PathBuf, IpcError> {
    match stream_id {
        Some(stream) => Ok(svc.worktrees.resolve_strict(Some(stream)).await?),
        None => Ok(svc.worktrees.project_dir().to_path_buf()),
    }
}

/// Load the bundle of the `custom` lens `id`'s component as it is now, for
/// its frame (tsk984): the version the daemon serves it at
/// (`/components/v/<version>/`) and the frame invokes with.
pub async fn load_component(
    svc: &Services,
    id: String,
    stream_id: Option<String>,
) -> Result<String, IpcError> {
    let root = component_root(svc, stream_id.as_deref()).await?;
    Ok(oxplow_app::lens_actions::load_component(svc, &root, &id)?)
}

/// A custom component's frame invokes one of its declared commands, as the
/// lens acting for the person (`NEEDS_CONFIRMATION`: the host asks them,
/// never the frame) — when the bundle `version` the frame was loaded at is
/// approved.
pub async fn invoke_component_command(
    svc: &Services,
    id: String,
    command: String,
    input: oxplow_domain::Json,
    stream_id: Option<String>,
    confirmed: bool,
    version: String,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    let root = component_root(svc, stream_id.as_deref()).await?;
    Ok(oxplow_app::lens_actions::invoke_component_command(
        svc,
        &root,
        oxplow_app::lens_actions::ComponentInvoke {
            lens_id: id,
            command,
            input: input.0,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed,
            version,
        },
    )
    .await?)
}

/// A person presses one of a lens's actions: its command runs as the
/// lens, acting for them (`NEEDS_CONFIRMATION` asks them first). A row
/// action takes the row it was pressed on.
pub async fn run_lens_action(
    svc: &Services,
    id: String,
    action: String,
    params: Option<BTreeMap<String, SqlCell>>,
    row: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
    confirmed: bool,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    let root = root(svc, stream_id.as_deref()).await;
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::run_lens_action(
        svc,
        &root,
        oxplow_app::lens_actions::LensActionCall {
            lens_id: id,
            action_id: action,
            params: params.unwrap_or_default(),
            row,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed,
        },
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
    oxplow_sdk::check(
        &root,
        &name,
        &svc.extension_catalog,
        Some(&svc.sql),
        Some(svc.commands.as_ref()),
        None,
    )
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
/// primary stream's extensions. UI only: an agent's `config.set` on the
/// person-only `extensions` key asks the person instead.
pub async fn set_extension_enabled(
    svc: &Services,
    name: String,
    enabled: bool,
) -> Result<Vec<Extension>, IpcError> {
    let mut disabled = oxplow_app::config_service::read_config(&svc.config).extensions_disabled;
    disabled.retain(|d| d != &name);
    if !enabled {
        disabled.push(name.clone());
    }
    let value = (!disabled.is_empty()).then(|| serde_json::json!({ "disabled": disabled }));
    crate::commands::config::set_key(svc, "extensions", value).await?;
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
    let commands = svc.commands.as_ref();
    Ok(match (git_url, name) {
        (Some(url), None) => {
            extensions::review_extension(
                &svc.sql,
                &svc.extension_catalog,
                &root,
                &url,
                git_ref.as_deref(),
                None,
                commands,
            )
            .await?
        }
        (None, Some(name)) => {
            extensions::review_update(&svc.sql, &svc.extension_catalog, &root, &name, commands)
                .await?
        }
        _ => {
            return Err(IpcError::invalid(
                "pass either gitUrl (install) or name (update)",
            ))
        }
    })
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
            "manifest: 2\nname: demo\nintent:\n  purpose: test\ndescription: Demo\n",
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
        write(
            repo.path(),
            "extension.yaml",
            "manifest: 2\nname: shared\nintent:\n  purpose: test\n",
        );
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
        // The install itself is the person's confirmed `extension.install`.
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = svc.clone();
            async move {
                svc.commands
                    .run(&oxplow_domain::Actor::Human, name, input, true)
                    .await
            }
        };
        let ext = run(
            "extension.install",
            json!({ "git_url": url, "reviewed_sha": sha }),
        )
        .await
        .unwrap()
        .result;
        assert_eq!(ext["name"], "shared");
        assert_eq!(ext["source"]["git"], json!(url));

        let err = run(
            "extension.install",
            json!({ "git_url": url, "reviewed_sha": sha }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already installed"), "{err}");

        write(
            repo.path(),
            "lenses/one.yaml",
            "title: One v2\nquery: SELECT 1\n",
        );
        git(repo.path(), &["commit", "-q", "-am", "v2"]);
        let review = crate::dispatch("review_extension", json!({ "name": "shared" }), &svc)
            .await
            .unwrap();
        let ext = run(
            "extension.update",
            json!({ "name": "shared", "reviewed_sha": review["sha"] }),
        )
        .await
        .unwrap()
        .result;
        assert_eq!(ext["lenses"][0]["title"], "One v2");
    }

    #[tokio::test]
    async fn extensions_can_be_disabled_and_reenabled() {
        let (svc, _dir) = crate::test_support::services();
        let enabled = |list: &serde_json::Value| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|e| e["name"] == "oxplow-bundled")
                .unwrap()["enabled"]
                .clone()
        };
        let list = crate::dispatch(
            "set_extension_enabled",
            json!({ "name": "oxplow-bundled", "enabled": false }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(enabled(&list), json!(false));
        let yaml =
            std::fs::read_to_string(svc.layout.project_dir.join(".oxplow/project.yaml")).unwrap();
        assert!(yaml.contains("oxplow-bundled"), "{yaml}");
        let list = crate::dispatch(
            "set_extension_enabled",
            json!({ "name": "oxplow-bundled", "enabled": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(enabled(&list), json!(true));
    }
}
