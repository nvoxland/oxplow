//! A page's actions, run by the open run they were pressed on. A whole-page
//! action binds the run's params; a node's row action runs on a row the
//! run holds (the presser names it; the server checks it is one of the
//! node's rows); a holds-one-row action on its node's one row. Each runs
//! through the bus as the page acting for whoever pressed it, with the
//! checks a lens's action passes (`lens_actions::prepare_resolved`).

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, CommandError, CommandOutcome};
use serde_json::Value;

use super::open_runs::PageRuns;
use super::runner::PageRun;
use super::text::{node, node_run};
use crate::lens_actions::{prepare_resolved, Drop};

/// One press of a page's action.
pub struct PageActionCall {
    /// The open run it was pressed on.
    pub run_id: String,
    /// The node whose action it is; `None` for a whole-page action. An
    /// embedded page's node is `<embed id>/<node id>`.
    pub node: Option<String>,
    pub action: String,
    /// For a row action, the row it was pressed on (column → value).
    pub row: Option<BTreeMap<String, SqlCell>>,
    /// For a form action, what the person filled in.
    pub input: Option<Value>,
    /// For a drag action, where the row landed.
    pub drop: Option<Drop>,
    pub on_behalf_of: Actor,
    pub confirmed: bool,
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

/// The run (an embedded one for `<embed>/<node>`) that holds `node`, and
/// the node's own id in it.
fn locate<'r>(run: &'r PageRun, path: &'r str) -> Option<(&'r PageRun, &'r str)> {
    match path.split_once('/') {
        None => Some((run, path)),
        Some((embed, rest)) => {
            let child = find_embed(&run.tree, embed)
                .or_else(|| run.rail.as_ref().and_then(|r| find_embed(r, embed)))?;
            locate(child, rest)
        }
    }
}

fn find_embed<'r>(at: &'r super::runner::RunNode, id: &str) -> Option<&'r PageRun> {
    if at.id == id {
        return at.embed.as_deref();
    }
    at.children.iter().find_map(|c| find_embed(c, id))
}

/// Run `call`'s action.
pub async fn run_page_action(
    svc: &crate::Services,
    runs: &PageRuns,
    call: PageActionCall,
) -> Result<CommandOutcome, CommandError> {
    let run = runs
        .get(&call.run_id)
        .ok_or_else(|| invalid("/runId", format!("no open page run `{}`", call.run_id)))?;
    let prepared = match &call.node {
        None => {
            // The page's own actions: a lens with them, over its params.
            let lens = crate::extensions::Lens {
                id: run.page.id.clone(),
                actions: run.page.actions.clone(),
                params: run.page.params.clone(),
                ..page_lens(&run)
            };
            if call.row.is_some() {
                return Err(invalid(
                    "/row",
                    format!("`{}` runs on the whole page, not a row", call.action),
                ));
            }
            prepare_resolved(
                &lens,
                &run.page.id,
                &call.action,
                &run.params,
                None,
                call.input,
                call.drop,
                call.on_behalf_of,
                call.confirmed,
            )?
        }
        Some(path) => {
            let (holder, id) = locate(&run, path)
                .ok_or_else(|| invalid("/node", format!("no node `{path}` in this run")))?;
            let def = node(&holder.page, id)
                .ok_or_else(|| invalid("/node", format!("no node `{path}` in this run")))?;
            let lens_run = node_run(holder, def)
                .ok_or_else(|| invalid("/node", format!("node `{path}` has no actions")))?;
            let action = lens_run
                .lens
                .actions
                .iter()
                .find(|a| a.id == call.action)
                .ok_or_else(|| {
                    invalid(
                        "/action",
                        format!("node `{path}` has no action `{}`", call.action),
                    )
                })?;
            let columns = &lens_run.result.columns;
            let as_map = |row: &Vec<SqlCell>| -> BTreeMap<String, SqlCell> {
                columns.iter().cloned().zip(row.iter().cloned()).collect()
            };
            let row = if action.one_row {
                if call.row.is_some() {
                    return Err(invalid(
                        "/row",
                        format!(
                            "`{}` runs on the node's one row, which the server reads",
                            call.action
                        ),
                    ));
                }
                match lens_run.result.rows.as_slice() {
                    [only] => Some(as_map(only)),
                    rows => {
                        return Err(invalid(
                            "/node",
                            format!(
                                "node `{path}` has {} rows; `{}` needs one",
                                rows.len(),
                                call.action
                            ),
                        ))
                    }
                }
            } else if let Some(given) = &call.row {
                // The row must be one the run holds, matched on the
                // columns the presser named.
                let held = lens_run
                    .result
                    .rows
                    .iter()
                    .map(as_map)
                    .find(|r| given.iter().all(|(k, v)| r.get(k) == Some(v)));
                match held {
                    Some(r) => Some(r),
                    None => {
                        return Err(invalid(
                            "/row",
                            format!(
                                "that row isn't one of node `{path}`'s rows now: the page changed"
                            ),
                        ))
                    }
                }
            } else {
                None
            };
            let actor = format!("{}#{id}", holder.page.id);
            prepare_resolved(
                &lens_run.lens,
                &actor,
                &call.action,
                &holder.params,
                row.as_ref(),
                call.input,
                call.drop,
                call.on_behalf_of,
                call.confirmed,
            )?
        }
    };
    prepared.run(svc).await
}

/// A lens standing for the page as a whole: what its own actions run as.
fn page_lens(run: &PageRun) -> crate::extensions::Lens {
    use super::schema::NodeKind;
    // Any viz node's lens carries the page's provenance and params; a page
    // with none still has its title.
    run.page
        .nodes()
        .into_iter()
        .find_map(|n| match &n.kind {
            NodeKind::Viz { lens, .. } => Some(lens.as_ref().clone()),
            _ => None,
        })
        .unwrap_or_else(|| empty_lens(run))
}

fn empty_lens(run: &PageRun) -> crate::extensions::Lens {
    crate::extensions::Lens {
        id: run.page.id.clone(),
        extension: run.page.extension.clone(),
        slug: run.page.slug.clone(),
        title: run.page.title.clone(),
        description: String::new(),
        query: String::new(),
        viz: crate::extensions::LensViz::Table,
        params: run.page.params.clone(),
        columns: Vec::new(),
        empty: None,
        chart: None,
        tree: None,
        tile: None,
        timeline: None,
        steps: None,
        hunks: None,
        code: None,
        diff: None,
        editor: None,
        embed: None,
        answer: None,
        terminal: None,
        markdown: None,
        form: None,
        custom: None,
        group: None,
        emphasis: None,
        depth: None,
        parts: Vec::new(),
        view: None,
        fills: false,
        launcher_category: None,
        hidden: true,
        refresh: false,
        actions: Vec::new(),
        alert: None,
        needs: Vec::new(),
        origin: run.page.origin.clone(),
        tool: None,
    }
}
