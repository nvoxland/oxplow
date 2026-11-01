//! Cores for the `pages` command module: a window opens a page run, hears
//! which of its loaders changed (`OxplowEvent::PageRunChanged`) and
//! fetches them, and closes it. Pages are the main worktree's, whichever
//! stream is open; a stream only scopes the data they read. See
//! `.context/pages-and-tabs.md` "Page files".

use std::collections::BTreeMap;

use oxplow_app::extension::extensions;

use oxplow_app::pages::open_runs::RunSnapshot;
use oxplow_app::pages::runner::{PageRun, PageRunner, RunRequest, RunTarget};
use oxplow_app::pages::schema::{parse_inline_page, Page};
use oxplow_app::pages::spec::ViewContext;
use oxplow_app::Services;
use oxplow_db::SqlCell;
use oxplow_domain::{Actor, DomainError};

use crate::error::IpcError;

/// A run a window opened: its id (what it closes, fetches and acts by)
/// and its first result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenedRun {
    pub run_id: String,
    pub run: PageRun,
}

/// What to run: a deployed page by id, or an inline page's YAML (a
/// markdown block's, an agent's one-off).
fn target(page: Option<String>, spec_yaml: Option<String>) -> Result<RunTarget, IpcError> {
    match (page, spec_yaml) {
        (Some(id), None) => Ok(RunTarget::Page(id)),
        (None, Some(yaml)) => Ok(RunTarget::Spec(std::sync::Arc::new(
            parse_inline_page(&yaml).map_err(DomainError::Invalid)?,
        ))),
        _ => Err(DomainError::Invalid("give `page` or `specYaml`, not both".into()).into()),
    }
}

async fn context(svc: &Services, stream_id: Option<&str>, thread_id: Option<&str>) -> ViewContext {
    oxplow_app::pages::spec::view_context(
        svc,
        stream_id.and_then(oxplow_domain::StreamId::try_from_str),
        thread_id.and_then(oxplow_domain::ThreadId::try_from_str),
    )
    .await
}

/// Run a page and keep it open for `client` (the window's id).
#[allow(clippy::too_many_arguments)]
pub async fn open_page(
    svc: &Services,
    page: Option<String>,
    spec_yaml: Option<String>,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
    thread_id: Option<String>,
    client: String,
) -> Result<OpenedRun, IpcError> {
    let req = RunRequest {
        target: target(page, spec_yaml)?,
        params: params.unwrap_or_default(),
        ctx: context(svc, stream_id.as_deref(), thread_id.as_deref()).await,
        viewer: Actor::Human,
    };
    let (run_id, run) = svc.page_runs.open(svc, req, &client).await?;
    Ok(OpenedRun { run_id, run })
}

pub async fn close_page(svc: &Services, run_id: String) -> Result<(), IpcError> {
    svc.page_runs.close(&run_id);
    Ok(())
}

/// Re-run an open run with new values for some of its params (a pick, a
/// filter, a switch's param).
pub async fn set_page_params(
    svc: &Services,
    run_id: String,
    params: BTreeMap<String, SqlCell>,
) -> Result<PageRun, IpcError> {
    Ok(svc.page_runs.set_params(svc, &run_id, params).await?.0)
}

/// Re-run an open run as it is (Refresh).
pub async fn rerun_page(svc: &Services, run_id: String) -> Result<PageRun, IpcError> {
    Ok(svc.page_runs.rerun(svc, &run_id).await?.0)
}

/// A window says whether a run's tab is on screen. Hidden runs are not
/// re-run on changes; shown with a change missed, it re-runs once (true:
/// the window fetches it).
pub async fn set_page_visible(
    svc: &Services,
    run_id: String,
    visible: bool,
) -> Result<bool, IpcError> {
    Ok(svc.page_runs.set_visible(svc, &run_id, visible).await?)
}

/// The open runs a window asks for, as they stand; one it no longer has
/// open (closed, or the daemon restarted) is left out.
pub async fn fetch_page_runs(
    svc: &Services,
    run_ids: Vec<String>,
) -> Result<BTreeMap<String, RunSnapshot>, IpcError> {
    Ok(run_ids
        .into_iter()
        .filter_map(|id| svc.page_runs.snapshot(&id).map(|r| (id, r)))
        .collect())
}

/// Run a page once, keeping nothing open (a launcher count, a picker's
/// options).
pub async fn run_page(
    svc: &Services,
    page: Option<String>,
    spec_yaml: Option<String>,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
) -> Result<PageRun, IpcError> {
    Ok(PageRunner::of(svc)
        .run(RunRequest {
            target: target(page, spec_yaml)?,
            params: params.unwrap_or_default(),
            ctx: context(svc, stream_id.as_deref(), None).await,
            viewer: Actor::Human,
        })
        .await?)
}

/// An open run as an agent reads it (Copy, Add to Context).
pub async fn page_text(svc: &Services, run_id: String) -> Result<String, IpcError> {
    let run = svc
        .page_runs
        .get(&run_id)
        .ok_or_else(|| DomainError::Invalid(format!("no open page run `{run_id}`")))?;
    Ok(oxplow_app::pages::text::page_text(svc, svc.worktrees.project_dir(), &run).await?)
}

/// One page's definition, by `<extension>/<slug>`.
pub async fn get_page(svc: &Services, id: String) -> Result<Page, IpcError> {
    Ok(svc
        .extension_catalog
        .find_page(svc.worktrees.project_dir(), &id)?)
}

/// The pages that can draw `uri`, in the order the extensions list them
/// (`oxplow_app::pages::renderers`): the window's one way to choose what
/// draws a tab and what Open With lists.
pub async fn renderers_for(
    svc: &Services,
    uri: String,
    stream_id: Option<String>,
) -> Result<Vec<oxplow_app::pages::renderers::Renderer>, IpcError> {
    let ctx = context(svc, stream_id.as_deref(), None).await;
    Ok(oxplow_app::pages::renderers::renderers_for(svc, &uri, &ctx, &Actor::Human).await)
}

/// The YAML a visual page edit starts from. Writable pages read their
/// primary source, including edits not yet deployed, so saving a tile
/// does not silently replace those edits with the deployed copy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageSpec {
    pub page: String,
    pub extension: String,
    pub slug: String,
    pub yaml: String,
    pub writable: bool,
}

pub async fn page_spec(svc: &Services, page: String) -> Result<PageSpec, IpcError> {
    let page = get_page(svc, page).await?;
    let root = svc.worktrees.project_dir();
    let source = extensions::writable_extension_dir(root, root, &page.extension)
        .ok()
        .map(|dir| dir.join("pages").join(format!("{}.yaml", page.slug)));
    let writable = source.as_ref().is_some_and(|path| path.is_file());
    let path = if writable {
        source.expect("a writable source exists")
    } else {
        root.join(&page.origin.running)
    };
    let yaml = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| DomainError::Storage(format!("read page `{}`: {e}", page.id)))?;
    Ok(PageSpec {
        page: page.id,
        extension: page.extension,
        slug: page.slug,
        yaml,
        writable,
    })
}

/// A press of one of an open run's actions: a whole-page action (no
/// `node`), or a node's (`<embed id>/<node id>` inside an embed), on `row`
/// for a row action. The server binds the run's params and checks the row
/// is one it holds.
#[allow(clippy::too_many_arguments)]
pub async fn run_page_action(
    svc: &Services,
    run_id: String,
    node: Option<String>,
    action: String,
    row: Option<BTreeMap<String, SqlCell>>,
    input: Option<oxplow_domain::Json>,
    drop: Option<oxplow_app::viz_actions::Drop>,
    confirmed: bool,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    use oxplow_app::pages::actions::{run_page_action, PageActionCall};
    Ok(run_page_action(
        svc,
        &svc.page_runs,
        PageActionCall {
            run_id,
            node,
            action,
            row,
            input: input.map(|j| j.0),
            drop,
            on_behalf_of: Actor::Human,
            confirmed,
        },
    )
    .await?)
}

/// The form a form node of an open run shows: its command, its starting
/// values, its choices and object fields.
pub async fn page_form(
    svc: &Services,
    run_id: String,
    node: String,
) -> Result<oxplow_app::viz_actions::FormStart, IpcError> {
    Ok(oxplow_app::pages::actions::page_form(svc, &svc.page_runs, &run_id, &node).await?)
}

/// A form node of an open run is submitted.
pub async fn submit_page_form(
    svc: &Services,
    run_id: String,
    node: String,
    input: oxplow_domain::Json,
    confirmed: bool,
) -> Result<oxplow_domain::CommandOutcome, IpcError> {
    Ok(oxplow_app::pages::actions::submit_page_form(
        svc,
        &svc.page_runs,
        &run_id,
        &node,
        input.0,
        Actor::Human,
        confirmed,
    )
    .await?)
}

/// The form a form action of an open run opens (on `row` for a row action).
pub async fn page_action_form(
    svc: &Services,
    run_id: String,
    node: Option<String>,
    action: String,
    row: Option<BTreeMap<String, SqlCell>>,
) -> Result<oxplow_app::viz_actions::FormStart, IpcError> {
    Ok(oxplow_app::pages::actions::page_action_form(
        svc,
        &svc.page_runs,
        &run_id,
        node.as_deref(),
        &action,
        row,
    )
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetched_page_runs_pair_the_current_result_with_its_actual_sequence() {
        let (ctx, _dir) = crate::test_support::services().await;
        let yaml = "title: Param\nparams: [{ name: n, default: 7 }]\nloaders:\n  value: { sql: 'SELECT :n AS n' }\nview: { viz: number, from: value }\n";
        let opened = open_page(
            &ctx,
            None,
            Some(yaml.into()),
            None,
            None,
            None,
            "snapshot-window".into(),
        )
        .await
        .unwrap();
        let before = fetch_page_runs(&ctx, vec![opened.run_id.clone()])
            .await
            .unwrap();
        let before = &before[&opened.run_id];
        let changed = set_page_params(
            &ctx,
            opened.run_id.clone(),
            BTreeMap::from([("n".into(), SqlCell::Int(9))]),
        )
        .await
        .unwrap();
        let fetched = fetch_page_runs(&ctx, vec![opened.run_id.clone(), "missing".into()])
            .await
            .unwrap();
        assert_eq!(fetched.len(), 1);
        let snapshot = &fetched[&opened.run_id];
        assert!(snapshot.seq > before.seq);
        assert_eq!(snapshot.run, changed);
        assert_eq!(
            snapshot.run.loaders["value"].result.rows,
            vec![vec![SqlCell::Int(9)]]
        );
        let wire = serde_json::to_value(&fetched).unwrap();
        assert_eq!(wire[&opened.run_id]["seq"], snapshot.seq);
        assert_eq!(
            wire[&opened.run_id]["run"],
            serde_json::to_value(&changed).unwrap()
        );
    }

    const INLINE: &str = "# Exact source\ntitle: An inline view\nloaders:\n  value: { sql: 'SELECT 7 AS n' }\nview: { viz: table, from: value }\n";

    /// A shown view is a page file on `oxplow:temp`: a window opens it as
    /// `temp/<id>` (a restarted daemon finds the file again), and one that
    /// is gone says so.
    #[tokio::test]
    async fn a_temp_view_opens_as_a_page_and_a_missing_one_says_so() {
        let (ctx, _dir) = crate::test_support::services().await;
        ctx.temp.put("v1.yaml", INLINE.as_bytes()).await.unwrap();
        let opened = open_page(
            &ctx,
            Some("temp/v1".into()),
            None,
            None,
            None,
            None,
            "window".into(),
        )
        .await
        .unwrap();
        assert_eq!(opened.run.page.id, "temp/v1");
        assert_eq!(
            opened.run.loaders["value"].result.rows,
            vec![vec![SqlCell::Int(7)]]
        );
        close_page(&ctx, opened.run_id).await.unwrap();
        ctx.temp.remove("v1.yaml").await.unwrap();
        let missing = open_page(
            &ctx,
            Some("temp/v1".into()),
            None,
            None,
            None,
            None,
            "window".into(),
        )
        .await
        .unwrap_err();
        assert_eq!(missing.code, "INVALID");
        assert!(missing.message.contains("temp/v1"), "{}", missing.message);
    }

    #[tokio::test]
    async fn page_spec_reads_undeployed_source_edits_and_marks_bundled_pages_read_only() {
        let (ctx, dir) = crate::test_support::services().await;
        let extension = dir.path().join(".oxplow/extensions/my-pages");
        std::fs::create_dir_all(extension.join("pages")).unwrap();
        std::fs::write(
            extension.join("extension.yaml"),
            "manifest: 2\nname: my-pages\nintent: { purpose: Pages for this project }\n",
        )
        .unwrap();
        std::fs::write(extension.join("pages/example.yaml"), INLINE).unwrap();
        crate::test_support::publish(&ctx).await;
        let edited = INLINE.replace("SELECT 7", "SELECT 8");
        std::fs::write(extension.join("pages/example.yaml"), &edited).unwrap();
        let source = page_spec(&ctx, "my-pages/example".into()).await.unwrap();
        assert!(source.writable);
        assert_eq!(source.yaml, edited);
        let deployed = run_page(&ctx, Some("my-pages/example".into()), None, None, None)
            .await
            .unwrap();
        assert_eq!(
            deployed.loaders["value"].result.rows,
            vec![vec![SqlCell::Int(7)]]
        );
        let bundled = page_spec(&ctx, "oxplow-foundation/commit".into())
            .await
            .unwrap();
        assert!(!bundled.writable);
        assert!(bundled.yaml.contains("title: Commit"));
    }
}
