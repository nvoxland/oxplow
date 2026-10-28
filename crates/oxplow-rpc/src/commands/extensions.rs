//! Cores for the `extensions` command module — project extensions and
//! their components. What the app shows is the main worktree's, whichever
//! stream is open. Checking an extension is the agent's
//! `oxplow.extension.validate` (the CLI's `check`), not an IPC read. See
//! `.context/extensions.md` "Reading".

use oxplow_app::extensions::Extension;
use oxplow_app::Services;
use oxplow_db::SqlCell;

use crate::error::IpcError;

/// Where the app's extensions come from: the main worktree, for every
/// stream. A stream's `stream_id` only scopes the data a page reads.
fn root(svc: &Services) -> &std::path::Path {
    svc.worktrees.project_dir()
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
    Ok(oxplow_app::viz_actions::run_component_sql(
        svc,
        root(svc),
        oxplow_app::viz_actions::ComponentSql {
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
) -> Result<oxplow_app::viz_actions::FormStart, IpcError> {
    Ok(oxplow_app::viz_actions::component_form_pinned(
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
    Ok(oxplow_app::viz_actions::load_component_pinned(svc, root(svc), &id).await?)
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
    Ok(oxplow_app::viz_actions::invoke_component_command(
        svc,
        root(svc),
        oxplow_app::viz_actions::ComponentInvoke {
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
            ".oxplow/extensions/demo/pages/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\n\
             data:\n  streams: { sql: \"SELECT kind FROM v_stream WHERE kind = :kind\" }\n\
             view: { viz: table, from: streams }\n",
        );
    }

    /// Loader `name`'s rows in a page run.
    fn rows(run: &serde_json::Value, name: &str) -> serde_json::Value {
        run["loaders"][name]["result"]["rows"].clone()
    }

    /// An inline page runs once; one that isn't a page is refused.
    #[tokio::test]
    async fn an_inline_page_runs_and_a_bad_one_is_refused() {
        let (svc, _dir) = crate::test_support::services().await;
        let run = crate::dispatch(
            "run_page",
            json!({ "specYaml": "title: Explore\ndata:\n  a: { sql: SELECT 1 AS n }\nview: { viz: number, from: a }\n" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(rows(&run, "a"), json!([[1]]));
        let err = crate::dispatch(
            "run_page",
            json!({ "specYaml": "title: Explore\nview: { viz: nonsense }\n" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    #[tokio::test]
    async fn lists_gets_runs_and_validates_pages() {
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
        assert_eq!(demo["pageFiles"][0]["id"], "demo/streams");

        let page = crate::dispatch("get_page", json!({ "id": "demo/streams" }), &svc)
            .await
            .unwrap();
        assert_eq!(page["title"], "Streams");

        let run = crate::dispatch("run_page", json!({ "page": "demo/streams" }), &svc)
            .await
            .unwrap();
        assert_eq!(rows(&run, "streams"), json!([["primary"]]));
        let run = crate::dispatch(
            "run_page",
            json!({ "page": "demo/streams", "params": { "kind": "worktree" } }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(rows(&run, "streams"), json!([]));
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
            "run_page",
            json!({ "page": "demo/streams", "streamId": stream }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(rows(&run, "streams"), json!([["primary"]]));
    }

    #[tokio::test]
    async fn a_missing_page_is_refused() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;
        let err = crate::dispatch("get_page", json!({ "id": "demo/nope" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
        let err = crate::dispatch("run_page", json!({ "page": "demo/nope" }), &svc)
            .await
            .unwrap_err();
        assert!(err.message.contains("demo/nope"), "{err:?}");
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
