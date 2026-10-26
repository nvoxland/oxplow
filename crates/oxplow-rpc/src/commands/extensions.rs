//! Cores for the `extensions` command module — project extensions and
//! their lenses. What the app shows is the main worktree's, whichever
//! stream is open; only the authoring call (`validate_extension`) reads a
//! stream's working copy. See
//! `.context/extensions.md` "Reading".

use std::collections::BTreeMap;

use oxplow_app::extensions::{self, Extension, Lens, LensRun};
use oxplow_app::Services;
use oxplow_db::SqlCell;
use oxplow_domain::DomainError;

use crate::error::IpcError;

/// Where the app's extensions come from: the main worktree, for every
/// stream. A stream's `stream_id` only scopes the data a lens reads.
fn root(svc: &Services) -> &std::path::Path {
    svc.worktrees.project_dir()
}

/// The working copy an authoring call checks: the stream's worktree, or
/// the main one when `stream_id` is omitted.
async fn working_copy(svc: &Services, stream_id: Option<&str>) -> std::path::PathBuf {
    svc.worktrees.resolve(stream_id).await.into_local_path()
}

/// Every extension the app shows, with per-extension load errors.
pub async fn list_extensions(svc: &Services) -> Result<Vec<Extension>, IpcError> {
    Ok(svc.listed_extensions(root(svc)).await)
}

/// A service a script may be given, as a person reads it
/// (`oxplow_domain::service::SERVICES`, the table every reader uses).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ServiceListing {
    /// What a script's `needs` names (`tree.read`).
    pub scope: String,
    /// "Read the captured tree".
    pub title: String,
    pub summary: String,
}

/// Every service, for the lines that say what an extension's scripts reach.
pub async fn list_services(_svc: &Services) -> Result<Vec<ServiceListing>, IpcError> {
    Ok(oxplow_domain::service::SERVICES
        .iter()
        .map(|s| ServiceListing {
            scope: s.scope.to_string(),
            title: s.title.to_string(),
            summary: s.summary.to_string(),
        })
        .collect())
}

/// One lens by `<extension>/<slug>`.
pub async fn get_lens(svc: &Services, id: String) -> Result<Lens, IpcError> {
    Ok(svc.lens_definition(root(svc), &id).await?)
}

/// Run a lens with optional param overrides; returns the rows the lens
/// page shows.
pub async fn run_lens(
    svc: &Services,
    id: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<LensRun, IpcError> {
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        root,
        &id,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?)
}

/// The viewer's context: the stream (primary when omitted) and its
/// selected thread, bound into lenses that declare `stream_id` /
/// `thread_id`.
async fn context(svc: &Services, stream_id: Option<&str>) -> extensions::ViewContext {
    let stream = stream_id.and_then(oxplow_domain::StreamId::try_from_str);
    extensions::lens_context(svc, stream, None).await
}

/// Run a lens spec a page shows without keeping it (Explore Data's
/// query), checked first; a refusal names its field (`/spec/query`).
pub async fn run_lens_spec(
    svc: &Services,
    spec: extensions::LensSpec,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<LensRun, IpcError> {
    let stream = stream_id
        .as_deref()
        .and_then(oxplow_domain::StreamId::try_from_str);
    Ok(
        oxplow_app::commands::lens::run_checked_spec(
            svc,
            &spec,
            params.unwrap_or_default(),
            stream,
        )
        .await?,
    )
}

/// Run an answer shown in a thread (`oxplow:oxplow/answer?id=<id>`) as the person sees it.
pub async fn run_answer(svc: &Services, answer: String) -> Result<LensRun, IpcError> {
    let id: i64 =
        oxplow_domain::uri::build::parse_as(&answer, oxplow_domain::uri::resources::ANSWER)
            .or_else(|_| answer.parse())
            .map_err(|_| {
                IpcError::invalid(format!(
                    "`{answer}` isn't an answer (`oxplow:oxplow/answer?id=<id>`)"
                ))
            })?;
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
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    let run = extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        root,
        &id,
        params.unwrap_or_default(),
        &ctx,
    )
    .await?;
    Ok(oxplow_app::lens_text::text_of(svc, root, &run, &ctx).await?)
}

/// What a form lens shows: its command's spec (the fields) and the values
/// they start from.
pub async fn lens_form(
    svc: &Services,
    id: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(
        oxplow_app::lens_actions::form_start(svc, root, &id, params.unwrap_or_default(), &ctx)
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
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::submit_form(
        svc,
        root,
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
/// `id` is the component (`<extension>/<component id>`); `asset` names the
/// lens to run.
pub async fn run_component_query(
    svc: &Services,
    id: String,
    asset: String,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
    version: String,
) -> Result<LensRun, IpcError> {
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::run_component_query(
        svc,
        root,
        &id,
        &asset,
        params.unwrap_or_default(),
        &ctx,
        &version,
    )
    .await?)
}

/// A custom component's frame runs its own read-only SQL (`querySql`):
/// `id` declares `sql: read`, its extension `permissions.sql`.
pub async fn run_component_sql(
    svc: &Services,
    id: String,
    sql: String,
    params: Option<Vec<SqlCell>>,
    stream_id: Option<String>,
    version: String,
) -> Result<oxplow_db::SqlQueryResult, IpcError> {
    let stream = stream_id
        .as_deref()
        .and_then(oxplow_domain::StreamId::try_from_str)
        .map(|s| s.value());
    Ok(oxplow_app::lens_actions::run_component_sql(
        svc,
        root(svc),
        oxplow_app::lens_actions::ComponentSql {
            component: id,
            sql,
            params: params.unwrap_or_default(),
            stream,
            limit: None,
            on_behalf_of: oxplow_domain::Actor::Human,
            version,
        },
    )
    .await?)
}

/// The form of one of component `id`'s commands, for the host to show
/// when its frame asks (`openForm`).
pub async fn component_form(
    svc: &Services,
    id: String,
    command: String,
    input: Option<oxplow_domain::Json>,
    version: String,
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
    Ok(oxplow_app::lens_actions::component_form_pinned(
        svc,
        root(svc),
        &id,
        &command,
        input.map(|i| i.0).unwrap_or(serde_json::Value::Null),
        &version,
    )
    .await?)
}

/// Load the bundle of component `id` (`<extension>/<component id>`) as it
/// is now, for its frame (tsk984): the version the daemon serves it at
/// (`/components/v/<version>/`) and the frame invokes with.
pub async fn load_component(svc: &Services, id: String) -> Result<String, IpcError> {
    Ok(oxplow_app::lens_actions::load_component_pinned(svc, root(svc), &id).await?)
}

/// A custom component's frame invokes one of its declared commands, as the
/// component acting for the person (`NEEDS_CONFIRMATION`: the host asks them,
/// never the frame) — when the bundle `version` the frame was loaded at is
/// approved.
pub async fn invoke_component_command(
    svc: &Services,
    id: String,
    command: String,
    input: oxplow_domain::Json,
    confirmed: bool,
    version: String,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    Ok(oxplow_app::lens_actions::invoke_component_command(
        svc,
        root(svc),
        oxplow_app::lens_actions::ComponentInvoke {
            component: id,
            command,
            input: input.0,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed,
            version,
        },
    )
    .await?)
}

/// One press of a lens's action: the lens, the action, the lens's params,
/// the row it was pressed on (a row action), and what the person filled in
/// (a form action, `form: true`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensPress {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub params: Option<BTreeMap<String, SqlCell>>,
    #[serde(default)]
    pub row: Option<BTreeMap<String, SqlCell>>,
    #[serde(default)]
    pub input: Option<oxplow_domain::Json>,
    /// Where a dragged row landed (a drag action, `drag: true`).
    #[serde(default)]
    pub drop: Option<oxplow_app::lens_actions::Drop>,
}

impl LensPress {
    fn call(self, confirmed: bool) -> oxplow_app::lens_actions::LensActionCall {
        oxplow_app::lens_actions::LensActionCall {
            lens_id: self.id,
            action_id: self.action,
            params: self.params.unwrap_or_default(),
            row: self.row,
            input: self.input.map(|i| i.0),
            drop: self.drop,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed,
        }
    }
}

/// The form a lens's form action (`form: true`) opens: its command, the
/// values it starts from (the action's bound `input`) and its fixed fields.
pub async fn lens_action_form(
    svc: &Services,
    press: LensPress,
    stream_id: Option<String>,
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::action_form(svc, root, press.call(false), &ctx).await?)
}

/// A person presses one of a lens's actions: its command runs as the
/// lens, acting for them (`NEEDS_CONFIRMATION` asks them first). A row
/// action takes the row it was pressed on; a form action, what they
/// filled in.
pub async fn run_lens_action(
    svc: &Services,
    press: LensPress,
    stream_id: Option<String>,
    confirmed: bool,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    let root = root(svc);
    let ctx = context(svc, stream_id.as_deref()).await;
    Ok(oxplow_app::lens_actions::run_lens_action(svc, root, press.call(confirmed), &ctx).await?)
}

/// Load one extension and dry-run each lens, returning every problem —
/// the SDK's `check`, the same report `oxplow extension check` prints.
pub async fn validate_extension(
    svc: &Services,
    name: String,
    stream_id: Option<String>,
) -> Result<oxplow_sdk::CheckReport, IpcError> {
    let root = working_copy(svc, stream_id.as_deref()).await;
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
            ".oxplow/extensions/demo/extension.yaml",
            "manifest: 2\nname: demo\nintent:\n  purpose: test\ndescription: Demo\n",
        );
        write(
            root,
            ".oxplow/extensions/demo/lenses/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        );
    }

    #[tokio::test]
    async fn a_spec_runs_checked_and_a_refusal_names_its_field() {
        let (svc, _dir) = crate::test_support::services().await;
        let run = crate::dispatch(
            "run_lens_spec",
            json!({ "spec": { "title": "Explore", "query": "SELECT 1 AS n", "viz": "number" } }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(run["result"]["rows"], json!([[1]]));
        let err = crate::dispatch(
            "run_lens_spec",
            json!({ "spec": { "title": "Explore", "query": "SELECT * FROM nope", "viz": "table" } }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
        assert_eq!(err.field.as_deref(), Some("/spec/query"));
    }

    #[tokio::test]
    async fn lists_gets_runs_and_validates_lenses() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;

        let exts = crate::dispatch("list_extensions", json!({}), &svc)
            .await
            .unwrap();
        let demo = exts
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "demo")
            .expect("listed");
        assert_eq!(demo["lenses"][0]["id"], "demo/streams");

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

    /// A worktree stream at `dir` (a git worktree of the project, adopted
    /// as a person would); its id.
    async fn worktree_stream(svc: &oxplow_app::Services, dir: &std::path::Path) -> String {
        git(
            &svc.layout.project_dir,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                &dir.to_string_lossy(),
            ],
        );
        let out = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.stream.create",
                json!({ "title": "Side", "source": "worktree", "worktree": dir.to_string_lossy() }),
                true,
            )
            .await
            .unwrap();
        let stream = out.result["stream"]
            .as_str()
            .or_else(|| out.result["id"].as_str())
            .unwrap_or_else(|| panic!("{}", out.result));
        stream.trim_start_matches("stream:").to_string()
    }

    /// The app shows the main worktree's extensions and honours its
    /// `project.yaml` whichever stream is open; a stream's own copy only
    /// shows once merged.
    #[tokio::test]
    async fn every_stream_shows_the_main_worktrees_extensions() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;
        write(
            &svc.layout.project_dir,
            ".oxplow/personal.yaml",
            "disableExtensions: [oxplow-bundled]\n",
        );
        crate::test_support::publish(&svc).await;
        let parent = tempfile::tempdir().unwrap();
        let side = parent.path().join("side");
        let stream = worktree_stream(&svc, &side).await;
        write(
            &side,
            ".oxplow/extensions/unmerged/extension.yaml",
            "manifest: 2\nname: unmerged\nintent:\n  purpose: test\n",
        );

        let exts = crate::dispatch("list_extensions", json!({ "streamId": stream }), &svc)
            .await
            .unwrap();
        let names: Vec<&str> = exts
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"demo"), "{names:?}");
        assert!(!names.contains(&"unmerged"), "{names:?}");
        let bundled = exts
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "oxplow-bundled")
            .unwrap();
        assert_eq!(bundled["enabled"], json!(false));

        let run = crate::dispatch(
            "run_lens",
            json!({ "id": "demo/streams", "streamId": stream }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(run["result"]["rows"], json!([["primary"]]));
    }

    #[tokio::test]
    async fn missing_lens_is_not_found_and_bad_params_are_invalid() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;
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
}
