//! A page's actions, run by the open run they were pressed on. A whole-page
//! action binds the run's params; a node's row action runs on a row the
//! run holds (the presser names it; the server checks it is one of the
//! node's rows); a holds-one-row action on its node's one row. Each runs
//! through the bus as the page acting for whoever pressed it, with the
//! checks a viz's action passes (`viz_actions::prepare_resolved`).

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, CommandError, CommandOutcome};
use serde_json::Value;

use super::open_runs::PageRuns;
use super::runner::PageRun;
use super::text::{node, node_run};
use crate::viz_actions::{prepare_resolved, Drop};

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

/// The embedded run an embed node at `path` holds (`<embed>`, or
/// `<embed>/<embed>` inside one); `None` when `path` names any other node.
fn embedded<'r>(run: &'r PageRun, path: &'r str) -> Option<&'r PageRun> {
    let (holder, id) = match path.rsplit_once('/') {
        None => (run, path),
        Some((outer, id)) => (locate(run, outer).and_then(|(h, i)| embedded_at(h, i))?, id),
    };
    embedded_at(holder, id)
}

fn embedded_at<'r>(run: &'r PageRun, id: &str) -> Option<&'r PageRun> {
    find_embed(&run.tree, id).or_else(|| run.rail.as_ref().and_then(|r| find_embed(r, id)))
}

/// The row a whole-page action `action` reads, when it reads one: the
/// page's first loader's one row.
fn page_row(
    run: &PageRun,
    action: &str,
) -> Result<Option<BTreeMap<String, SqlCell>>, CommandError> {
    let reads_row = run.page.actions.iter().any(|a| a.id == action && a.one_row);
    if !reads_row {
        return Ok(None);
    }
    let first = run
        .page
        .loaders
        .first()
        .and_then(|l| run.loaders.get(&l.name))
        .map(|l| &l.result);
    match first.map(|r| (r, r.rows.as_slice())) {
        Some((r, [only])) => Ok(Some(
            r.columns
                .iter()
                .cloned()
                .zip(only.iter().cloned())
                .collect(),
        )),
        Some((_, rows)) => Err(invalid(
            "/action",
            format!(
                "`{action}` reads the page's row, but its first loader has {} rows",
                rows.len()
            ),
        )),
        None => Err(invalid(
            "/action",
            format!("`{action}` reads the page's row, but it has no loader"),
        )),
    }
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
    // A whole-page action: the page's own, or an embedded page's named by
    // its embed node.
    let whole = match &call.node {
        None => Some(&run),
        Some(path) => embedded(&run, path),
    };
    let prepared = match (&call.node, whole) {
        (_, Some(target)) => {
            if call.row.is_some() {
                return Err(invalid(
                    "/row",
                    format!("`{}` runs on the whole page, not a row", call.action),
                ));
            }
            let row = page_row(target, &call.action)?;
            prepare_resolved(
                &target.page.id,
                &target.page.actions,
                &target.page.id,
                &call.action,
                &target.params,
                row.as_ref(),
                call.input,
                call.drop,
                call.on_behalf_of,
                call.confirmed,
            )?
        }
        (None, None) => unreachable!("no node is the page itself"),
        (Some(path), None) => {
            let (holder, id) = locate(&run, path)
                .ok_or_else(|| invalid("/node", format!("no node `{path}` in this run")))?;
            let def = node(&holder.page, id)
                .ok_or_else(|| invalid("/node", format!("no node `{path}` in this run")))?;
            let viz_run = node_run(holder, def)
                .ok_or_else(|| invalid("/node", format!("node `{path}` has no actions")))?;
            let action = viz_run
                .config
                .actions
                .iter()
                .find(|a| a.id == call.action)
                .ok_or_else(|| {
                    invalid(
                        "/action",
                        format!("node `{path}` has no action `{}`", call.action),
                    )
                })?;
            let columns = &viz_run.result.columns;
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
                match viz_run.result.rows.as_slice() {
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
                let held = viz_run
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
                &viz_run.config.id,
                &viz_run.config.actions,
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

/// The rows a form's choice or object field reads: page `source`
/// (`<extension>/<slug>`, its first loader, or `…#<loader>`), run with the
/// form's params that page declares.
async fn page_rows(
    svc: &crate::Services,
    source: &str,
    params: &BTreeMap<String, SqlCell>,
    viewer: &Actor,
) -> Result<oxplow_db::SqlQueryResult, CommandError> {
    let (page_id, loader) = match source.split_once('#') {
        Some((p, l)) => (p, Some(l)),
        None => (source, None),
    };
    let page = svc
        .extension_catalog
        .find_page(&svc.layout.project_dir, page_id)
        .map_err(|_| invalid("/source", format!("no page `{page_id}`")))?;
    let given = params
        .iter()
        .filter(|(k, _)| page.params.iter().any(|p| &p.name == *k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let name = match loader {
        Some(l) => l.to_string(),
        None => page
            .loaders
            .first()
            .map(|l| l.name.clone())
            .ok_or_else(|| invalid("/source", format!("page `{page_id}` has no loader")))?,
    };
    let run = super::runner::PageRunner::of(svc)
        .run(super::runner::RunRequest {
            target: super::runner::RunTarget::Spec(std::sync::Arc::new(page)),
            params: given,
            ctx: crate::pages::spec::ViewContext::default(),
            viewer: viewer.clone(),
        })
        .await
        .map_err(CommandError::from)?;
    let l = run.loaders.get(&name).ok_or_else(|| {
        invalid(
            "/source",
            format!("page `{page_id}` ran no loader `{name}`"),
        )
    })?;
    match &l.error {
        Some(e) => Err(invalid("/source", e.clone())),
        None => Ok(l.result.clone()),
    }
}

/// Each choice field's choices and each object field's children, from the
/// pages they name.
async fn form_sources(
    svc: &crate::Services,
    choices: &BTreeMap<String, String>,
    fields: &[crate::pages::spec::FormField],
    params: &BTreeMap<String, SqlCell>,
    viewer: &Actor,
) -> Result<
    (
        BTreeMap<String, Vec<crate::viz_actions::Choice>>,
        BTreeMap<String, Vec<crate::viz_actions::Subfield>>,
        oxplow_db::Reads,
    ),
    CommandError,
> {
    let mut out_choices = BTreeMap::new();
    for (field, source) in choices {
        let rows = page_rows(svc, source, params, viewer).await?;
        out_choices.insert(
            field.clone(),
            crate::viz_actions::choices_from(&rows, field, source)?,
        );
    }
    let mut subfields = BTreeMap::new();
    let mut reads = Vec::new();
    for field in fields {
        let Some(source) = field.from() else { continue };
        let rows = page_rows(svc, source, params, viewer).await?;
        reads.push(rows.reads.clone());
        subfields.insert(
            field.name().to_string(),
            crate::viz_actions::subfields_from(&rows, field.name(), source)?,
        );
    }
    Ok((out_choices, subfields, oxplow_db::Reads::union(&reads)))
}

/// The form node `node` of open run `run_id` (a `form` viz): its command,
/// its defaults (params bound) under its loader's first row, its choices
/// and object fields read from the pages they name.
pub async fn page_form(
    svc: &crate::Services,
    runs: &PageRuns,
    run_id: &str,
    node_path: &str,
) -> Result<crate::viz_actions::FormStart, CommandError> {
    let run = runs
        .get(run_id)
        .ok_or_else(|| invalid("/runId", format!("no open page run `{run_id}`")))?;
    let (holder, id) = locate(&run, node_path)
        .ok_or_else(|| invalid("/node", format!("no node `{node_path}`")))?;
    let def =
        node(&holder.page, id).ok_or_else(|| invalid("/node", format!("no node `{node_path}`")))?;
    let viz_run = node_run(holder, def)
        .ok_or_else(|| invalid("/node", format!("`{node_path}` isn't a viz")))?;
    let form = match (&viz_run.config.viz, &viz_run.config.form) {
        (crate::pages::spec::Viz::Form, Some(f)) => f.clone(),
        _ => return Err(invalid("/node", format!("node `{node_path}` isn't a form"))),
    };
    let name = form.command.clone().unwrap_or_default();
    let command = svc
        .commands
        .spec(&name)
        .ok_or_else(|| CommandError::Unknown { name: name.clone() })?;
    let mut values = match form.defaults.as_ref() {
        Some(d) => match crate::viz_actions::bind_input(d, &holder.params, None, None)? {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        },
        None => serde_json::Map::new(),
    };
    if let Some(row) = viz_run.result.rows.first() {
        for (col, cell) in viz_run.result.columns.iter().zip(row) {
            if *cell != SqlCell::Null(()) {
                values.insert(
                    col.clone(),
                    serde_json::to_value(cell).unwrap_or(Value::Null),
                );
            }
        }
    }
    for field in form.fields.iter().flatten().filter(|f| f.from().is_some()) {
        if let Some(Value::String(text)) = values.get(field.name()) {
            if let Ok(object @ Value::Object(_)) = serde_json::from_str::<Value>(text) {
                values.insert(field.name().to_string(), object);
            }
        }
    }
    for field in form.fields.iter().flatten().filter(|f| f.secret()) {
        let kept = command
            .unrecorded
            .iter()
            .any(|k| k == field.name() || k == "*");
        if !kept {
            return Err(invalid(
                "/node",
                format!(
                    "`{}` is secret but `{}` records it: the command must declare it `unrecorded`",
                    field.name(),
                    command.id
                ),
            ));
        }
        values.remove(field.name());
    }
    let (choices, subfields, reads) = form_sources(
        svc,
        &form.choices,
        form.fields.as_deref().unwrap_or_default(),
        &holder.params,
        &Actor::Human,
    )
    .await?;
    Ok(crate::viz_actions::FormStart {
        command,
        values: Value::Object(values),
        fixed: form.fixed,
        fields: form.fields,
        choices,
        subfields,
        reads,
    })
}

/// A form node is submitted: its command runs as the page for whoever
/// submitted, the form's defaults under what they filled in.
pub async fn submit_page_form(
    svc: &crate::Services,
    runs: &PageRuns,
    run_id: &str,
    node_path: &str,
    input: Value,
    on_behalf_of: Actor,
    confirmed: bool,
) -> Result<CommandOutcome, CommandError> {
    let run = runs
        .get(run_id)
        .ok_or_else(|| invalid("/runId", format!("no open page run `{run_id}`")))?;
    let (holder, id) = locate(&run, node_path)
        .ok_or_else(|| invalid("/node", format!("no node `{node_path}`")))?;
    let def =
        node(&holder.page, id).ok_or_else(|| invalid("/node", format!("no node `{node_path}`")))?;
    let viz_run = node_run(holder, def)
        .ok_or_else(|| invalid("/node", format!("`{node_path}` isn't a viz")))?;
    let form = match (&viz_run.config.viz, &viz_run.config.form) {
        (crate::pages::spec::Viz::Form, Some(f)) => f.clone(),
        _ => return Err(invalid("/node", format!("node `{node_path}` isn't a form"))),
    };
    let defaults = match form.defaults.as_ref() {
        Some(d) => crate::viz_actions::bind_input(d, &holder.params, None, None)?,
        None => Value::Object(serde_json::Map::new()),
    };
    let merged = crate::viz_actions::filled(defaults, input, &form.fixed)?;
    svc.commands
        .run(
            &Actor::Page {
                page_id: format!("{}#{id}", holder.page.id),
                on_behalf_of: Box::new(on_behalf_of),
            },
            &form.command.unwrap_or_default(),
            merged,
            confirmed,
        )
        .await
}

/// The form a form action (`form: true`) opens: its command, its input
/// bound (params, and the row it is pressed on) as the starting values, its
/// `fixed` fields and its choices.
pub async fn page_action_form(
    svc: &crate::Services,
    runs: &PageRuns,
    run_id: &str,
    node_path: Option<&str>,
    action_id: &str,
    row: Option<BTreeMap<String, SqlCell>>,
) -> Result<crate::viz_actions::FormStart, CommandError> {
    let run = runs
        .get(run_id)
        .ok_or_else(|| invalid("/runId", format!("no open page run `{run_id}`")))?;
    let (actions, params, rows): (
        Vec<crate::pages::spec::PageAction>,
        BTreeMap<String, SqlCell>,
        Option<oxplow_db::SqlQueryResult>,
    ) = match (
        node_path,
        node_path.map_or(Some(&run), |p| embedded(&run, p)),
    ) {
        (_, Some(target)) => {
            // A whole-page form action starts from the page's row, when it
            // reads one.
            let one = page_row(target, action_id)?;
            if row.is_none() && one.is_some() {
                let action = target
                    .page
                    .actions
                    .iter()
                    .find(|a| a.id == action_id && a.form)
                    .cloned()
                    .ok_or_else(|| invalid("/action", format!("no form action `{action_id}`")))?;
                return form_start(svc, action, &target.params, one.as_ref()).await;
            }
            (target.page.actions.clone(), target.params.clone(), None)
        }
        (None, None) => unreachable!("no node is the page itself"),
        (Some(path), None) => {
            let (holder, id) =
                locate(&run, path).ok_or_else(|| invalid("/node", format!("no node `{path}`")))?;
            let def = node(&holder.page, id)
                .ok_or_else(|| invalid("/node", format!("no node `{path}`")))?;
            let viz_run = node_run(holder, def)
                .ok_or_else(|| invalid("/node", format!("`{path}` has no actions")))?;
            (
                viz_run.config.actions,
                holder.params.clone(),
                Some(viz_run.result),
            )
        }
    };
    let action = actions
        .iter()
        .find(|a| a.id == action_id && a.form)
        .cloned()
        .ok_or_else(|| invalid("/action", format!("no form action `{action_id}`")))?;
    // A one-row action reads its node's one row, which the server holds.
    let one_row = match (&row, &rows) {
        (None, Some(result)) if action.one_row => match result.rows.as_slice() {
            [only] => Some(
                result
                    .columns
                    .iter()
                    .cloned()
                    .zip(only.iter().cloned())
                    .collect::<BTreeMap<_, _>>(),
            ),
            rows => {
                return Err(invalid(
                    "/action",
                    format!(
                        "`{action_id}` runs on its node's one row, but it has {}",
                        rows.len()
                    ),
                ))
            }
        },
        _ => None,
    };
    if one_row.is_some() {
        return form_start(svc, action, &params, one_row.as_ref()).await;
    }
    // A row the presser names must be one the run holds.
    let row = match (row, rows) {
        (Some(given), Some(result)) => Some(
            result
                .rows
                .iter()
                .map(|r| {
                    result
                        .columns
                        .iter()
                        .cloned()
                        .zip(r.iter().cloned())
                        .collect::<BTreeMap<_, _>>()
                })
                .find(|r| given.iter().all(|(k, v)| r.get(k) == Some(v)))
                .ok_or_else(|| {
                    invalid("/row", "that row isn't one the page holds now".to_string())
                })?,
        ),
        (Some(_), None) => {
            return Err(invalid(
                "/row",
                format!("`{action_id}` runs on the whole page"),
            ))
        }
        (None, _) => None,
    };
    form_start(svc, action, &params, row.as_ref()).await
}

/// A form action's form: its input bound over `params` and `row`, its
/// choices read.
async fn form_start(
    svc: &crate::Services,
    action: crate::pages::spec::PageAction,
    params: &BTreeMap<String, SqlCell>,
    row: Option<&BTreeMap<String, SqlCell>>,
) -> Result<crate::viz_actions::FormStart, CommandError> {
    let values = crate::viz_actions::bind_input(&action.input, params, row, None)?;
    let (choices, _, _) = form_sources(svc, &action.choices, &[], params, &Actor::Human).await?;
    let command = svc
        .commands
        .spec(&action.command)
        .ok_or_else(|| CommandError::Unknown {
            name: action.command.clone(),
        })?;
    Ok(crate::viz_actions::FormStart {
        command,
        values,
        fixed: action.fixed,
        fields: None,
        choices,
        subfields: BTreeMap::new(),
        reads: oxplow_db::Reads::default(),
    })
}
