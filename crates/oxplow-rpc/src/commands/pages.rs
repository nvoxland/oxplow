//! Cores for the `pages` command module: a window opens a page run, hears
//! which of its loaders changed (`OxplowEvent::PageRunChanged`) and
//! fetches them, and closes it. Pages are the main worktree's, whichever
//! stream is open; a stream only scopes the data they read. See
//! `.context/pages-and-tabs.md` "Page files".

use std::collections::BTreeMap;

use oxplow_app::extensions::{self, LensContext};
use oxplow_app::pages::runner::{PageRun, PageRunner, RunRequest, RunTarget};
use oxplow_app::pages::schema::{parse_inline_page, Page};
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

async fn context(svc: &Services, stream_id: Option<&str>, thread_id: Option<&str>) -> LensContext {
    extensions::lens_context(
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
    view: Option<String>,
    params: Option<BTreeMap<String, SqlCell>>,
    stream_id: Option<String>,
    thread_id: Option<String>,
    client: String,
) -> Result<OpenedRun, IpcError> {
    // A view an agent showed: its page, as it was kept.
    let spec_yaml = match view {
        Some(id) => Some(
            svc.page_runs
                .view(&id)
                .ok_or_else(|| {
                    DomainError::Invalid(format!("no view `{id}` (it went with a restart)"))
                })?
                .spec_yaml,
        ),
        None => spec_yaml,
    };
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

/// The open runs a window asks for, as they stand; one it no longer has
/// open (closed, or the daemon restarted) is left out.
pub async fn fetch_page_runs(
    svc: &Services,
    run_ids: Vec<String>,
) -> Result<BTreeMap<String, PageRun>, IpcError> {
    Ok(run_ids
        .into_iter()
        .filter_map(|id| svc.page_runs.get(&id).map(|r| (id, r)))
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
    Ok(oxplow_app::pages::text::page_text(
        svc,
        svc.worktrees.project_dir(),
        &run,
        &LensContext::default(),
    )
    .await?)
}

/// One page's definition, by `<extension>/<slug>`.
pub async fn get_page(svc: &Services, id: String) -> Result<Page, IpcError> {
    Ok(svc
        .extension_catalog
        .find_page(svc.worktrees.project_dir(), &id)?)
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
    drop: Option<oxplow_app::lens_actions::Drop>,
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
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
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
) -> Result<oxplow_app::lens_actions::FormStart, IpcError> {
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
