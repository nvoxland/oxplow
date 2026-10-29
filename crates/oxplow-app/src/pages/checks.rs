//! A check's dry run of a page: its `sql` loaders run once with their
//! params' defaults (earlier loaders read as tables), so a query that
//! fails, and a viz node naming a column its loader doesn't return, are
//! found before the page is deployed. `command` and `page` loaders aren't
//! run (a check acts as no one); a node they feed isn't column-checked.

use std::collections::BTreeMap;

use oxplow_db::{SqlCell, SqlQueryResult, TempTable};

use super::runner::{bind_args, LoaderRun};
use super::schema::{Loader, NodeKind, Page};
use crate::pages::spec::{bind_params, VizConfig, VizRun};

/// Each loader of a page that ran, by name: its result or why it failed.
pub type LoaderChecks = BTreeMap<String, Result<SqlQueryResult, String>>;

/// Each page of an extension, by slug: its loaders' dry runs.
pub type PageChecks = BTreeMap<String, LoaderChecks>;

/// Dry-run `page`'s `sql` loaders through `layer`.
pub async fn dry_run(layer: &crate::sql_gateway::SqlGateway, page: &Page) -> LoaderChecks {
    // Each param at its default (else NULL), as a run with none given.
    let params: BTreeMap<String, SqlCell> = crate::pages::spec::resolve_param_values(
        &page.id,
        &page.params,
        &BTreeMap::new(),
        &crate::pages::spec::ViewContext::default(),
    )
    .unwrap_or_default();
    let mut earlier: BTreeMap<String, LoaderRun> = BTreeMap::new();
    let mut out = LoaderChecks::new();
    for decl in &page.loaders {
        let Loader::Sql { sql, params: given } = &decl.loader else {
            continue;
        };
        // One that reads a loader that didn't run can't run either.
        if decl.reads_loaders.iter().any(|n| !earlier.contains_key(n)) {
            continue;
        }
        let temp = decl
            .reads_loaders
            .iter()
            .filter_map(|n| {
                let r = &earlier.get(n)?.result;
                Some(TempTable {
                    name: n.clone(),
                    columns: r.columns.clone(),
                    rows: r.rows.clone(),
                })
            })
            .collect::<Vec<_>>();
        let mut named = bind_params(&page.params, &params, time::OffsetDateTime::now_utc());
        for (k, v) in bind_args(given, &params, &earlier) {
            named.retain(|(n, _)| n != &k);
            named.push((k, SqlCell::from(v)));
        }
        let mut query = oxplow_db::SqlQuery::new(sql).named(named).limit(Some(200));
        query.temp = temp;
        match layer.run(query).await {
            Ok(result) => {
                earlier.insert(
                    decl.name.clone(),
                    LoaderRun {
                        result: result.clone(),
                        error: None,
                        key: String::new(),
                    },
                );
                out.insert(decl.name.clone(), Ok(result));
            }
            Err(e) => {
                out.insert(decl.name.clone(), Err(e.to_string()));
            }
        }
    }
    out
}

/// Every page of `ext`, dry-run.
pub async fn dry_run_all(
    layer: &crate::sql_gateway::SqlGateway,
    ext: &crate::extension::extensions::Extension,
) -> PageChecks {
    let mut out = PageChecks::new();
    for page in &ext.page_files {
        out.insert(page.slug.clone(), dry_run(layer, page).await);
    }
    out
}

/// Each viz node of `page` with the loader it shows, when that loader ran.
pub fn viz_nodes<'p>(
    page: &'p Page,
    loaders: &'p LoaderChecks,
) -> Vec<(&'p str, &'p VizConfig, &'p Result<SqlQueryResult, String>)> {
    page.nodes()
        .into_iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Viz {
                from: Some(from),
                config: cfg,
            } => loaders.get(from).map(|r| (n.id.as_str(), cfg.as_ref(), r)),
            _ => None,
        })
        .collect()
}

/// What `page` shows from its dry run, as text: each viz node under its
/// title — what a review compares between two versions.
pub fn text(page: &Page, loaders: &LoaderChecks) -> Result<String, String> {
    let mut parts = Vec::new();
    for (id, cfg, result) in viz_nodes(page, loaders) {
        let result = result.as_ref().map_err(|e| format!("node `{id}`: {e}"))?;
        let run = VizRun {
            config: cfg.clone(),
            params: BTreeMap::new(),
            result: result.clone(),
        };
        let body = crate::viz_text::render(&run, &crate::viz_text::Resolved::default());
        parts.push(format!("### {}\n\n{body}", cfg.title));
    }
    Ok(parts.join("\n\n"))
}

/// The most `filter` params a page draws, `choice` options a select
/// lists, characters a description runs, sections a stack holds and
/// columns a table shows before the layout warnings say so.
const MAX_FILTERS: usize = 6;
const MAX_CHOICE_OPTIONS: usize = 12;
const MAX_DESCRIPTION: usize = 200;
const MAX_SECTIONS: usize = 8;
const MAX_COLUMNS: usize = 8;

/// The params of a page shown in the launcher (it has a `category` and isn't
/// `hidden`) that nothing fills in when it opens there: no default, no
/// options from a loader, and neither a filter nor a selection (which start
/// empty on purpose). The viewer's `stream_id`, `thread_id` and `head_sha`
/// are filled in for it.
pub fn launcher_unfilled_params(page: &Page) -> Vec<String> {
    use crate::pages::spec::ParamRole;
    // A resource page (`for:`) is opened from its resource, never the launcher.
    if page.hidden || page.category.is_none() || page.is_resource_page() {
        return Vec::new();
    }
    page.params
        .iter()
        .filter(|p| {
            p.default.is_none()
                && p.options_from.is_none()
                && !matches!(p.role, Some(ParamRole::Selection | ParamRole::Filter))
                && !["stream_id", "thread_id", "head_sha"].contains(&p.name.as_str())
        })
        .map(|p| p.name.clone())
        .collect()
}

/// What makes `page` harder to read than it needs to be: too many filters,
/// options, sections or columns, a long description, a param nothing reads.
/// Warnings, never errors.
pub fn layout_warnings(page: &Page) -> Vec<String> {
    use crate::pages::spec::{ParamRole, ParamType};
    let mut out = Vec::new();
    let nodes = page.nodes();
    let switches: Vec<&str> = nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Switch { param, .. } => Some(param.as_str()),
            _ => None,
        })
        .collect();
    let filters = page
        .params
        .iter()
        .filter(|p| p.role == Some(ParamRole::Filter) && !switches.contains(&p.name.as_str()))
        .count();
    if filters > MAX_FILTERS {
        out.push(format!(
            "{filters} filters; a page shows {MAX_FILTERS} at most — fold related ones into one `choice` (not several `bool`s), or split the page"
        ));
    }
    for p in &page.params {
        if p.kind == Some(ParamType::Choice) && p.options.len() > MAX_CHOICE_OPTIONS {
            out.push(format!(
                "param `{}` has {} options — use a `search`, or a page loader that lists them (`options_from`)",
                p.name,
                p.options.len()
            ));
        }
    }
    // A param flows by name into what a page embeds or mounts, so only a
    // page without either can say one is never read.
    // A replaceable node is given them too: a replacement takes the page's
    // params as its props.
    let passes_on = nodes.iter().any(|n| n.replaceable.is_some())
        || nodes.iter().any(|n| match &n.kind {
            NodeKind::Embed { .. } | NodeKind::Slot { .. } | NodeKind::Component { .. } => true,
            // A custom viz is a component too: it is given the page's params.
            NodeKind::Viz { config, .. } => config.viz == crate::pages::spec::Viz::Custom,
            _ => false,
        });
    if !passes_on {
        let mut read: Vec<String> = page
            .loaders
            .iter()
            .flat_map(|l| l.reads_params.iter().cloned())
            .collect();
        read.extend(switches.iter().map(|s| s.to_string()));
        let mut actions: Vec<&crate::pages::spec::PageAction> = page.actions.iter().collect();
        for n in &nodes {
            if let NodeKind::Viz { config: cfg, .. } = &n.kind {
                actions.extend(cfg.actions.iter());
                read.extend(
                    cfg.columns
                        .iter()
                        .flat_map(|c| [c.select.clone(), c.highlight.clone()])
                        .flatten(),
                );
                if cfg.form.is_some() {
                    // A form's defaults bind the params it is opened with.
                    read.extend(page.params.iter().map(|p| p.name.clone()));
                }
            }
        }
        for a in actions {
            read.extend(
                crate::pages::spec::action_templates(&a.input)
                    .into_iter()
                    .filter(|(scope, _)| scope == "param")
                    .map(|(_, name)| name),
            );
        }
        if let Some(label) = &page.label {
            read.extend(
                page.params
                    .iter()
                    .map(|p| p.name.clone())
                    .filter(|n| label.contains(&format!("param.{n}"))),
            );
        }
        let renderer = !page.renders.is_empty();
        for p in &page.params {
            let range = p.kind == Some(ParamType::DateRange);
            let is_read = read.contains(&p.name)
                || range
                    && (read.contains(&format!("{}_from", p.name))
                        || read.contains(&format!("{}_to", p.name)))
                // A renderer is run with the stream and thread it is shown in:
                // the host's to give (its `uri` is read by `content`).
                || renderer && ["stream_id", "thread_id"].contains(&p.name.as_str());
            if !is_read {
                out.push(format!(
                    "param `{0}` isn't read by any loader of the page — remove it, or read it as `:{0}`",
                    p.name
                ));
            }
        }
    }
    let described = page.description.chars().count();
    if described > MAX_DESCRIPTION && !page.hidden {
        out.push(format!(
            "the description is {described} characters — keep it to a sentence or two; put the rest in a markdown node"
        ));
    }
    if let NodeKind::Stack { children } = &page.view.kind {
        if children.len() > MAX_SECTIONS {
            out.push(format!(
                "{} sections — a page reads best with {MAX_SECTIONS} at most; split it",
                children.len()
            ));
        }
    }
    if page.tool.is_none() {
        for n in &nodes {
            if let NodeKind::Viz { config: cfg, .. } = &n.kind {
                if matches!(
                    cfg.viz,
                    crate::pages::spec::Viz::Table | crate::pages::spec::Viz::List
                ) && cfg.columns.len() > MAX_COLUMNS
                {
                    out.push(format!(
                        "node `{}`: {} columns — show {MAX_COLUMNS} at most; put the rest in a node a row `select`s",
                        n.id,
                        cfg.columns.len()
                    ));
                }
            }
        }
    }
    out
}
