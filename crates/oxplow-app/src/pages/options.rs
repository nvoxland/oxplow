//! The rows a reference to a page file means: a param's `options_from`, a
//! form's `choices` or field `from`. Each names `<ext>/<slug>` (its default
//! loader), `<ext>/<slug>#<loader>`, or — qualified at load from a bare
//! `#<loader>` — a loader of the page the reference is on. Only that
//! loader's chain runs, with the params the target declares, so a page's
//! option list costs the page nothing when the rail refreshes it.

use std::collections::BTreeMap;

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::{Actor, DomainError};

use super::refs::{self, Shape};
use super::runner::PageRunner;
use super::schema::Page;
use super::spec::ViewContext;
use crate::Services;

/// The page file `target` names and the loader it means.
pub fn resolve(svc: &Services, target: &str) -> Result<(Page, String), DomainError> {
    let (id, loader) = refs::split(target);
    let page = svc
        .extension_catalog
        .find_page(&svc.layout.project_dir, id)
        .map_err(|_| DomainError::Invalid(format!("no page `{id}`")))?;
    let name = match loader {
        Some(l) => {
            if page.loader(l).is_none() {
                return Err(DomainError::Invalid(format!("`{id}` has no loader `{l}`")));
            }
            l.to_string()
        }
        None => page
            .default_loader()
            .map(|l| l.name.clone())
            .ok_or_else(|| DomainError::Invalid(format!("`{id}` has no loader")))?,
    };
    Ok((page, name))
}

/// The rows `target` means, run with the values of `supplied` the target
/// declares as params, for `ctx` as `viewer`: only that loader's chain.
pub async fn source_rows(
    svc: &Services,
    target: &str,
    supplied: &BTreeMap<String, SqlCell>,
    ctx: &ViewContext,
    viewer: &Actor,
) -> Result<SqlQueryResult, DomainError> {
    let (page, loader) = resolve(svc, target)?;
    let given = supplied
        .iter()
        .filter(|(k, _)| page.params.iter().any(|p| &p.name == *k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    PageRunner::of(svc)
        .run_loader_chain(&page, &loader, given, ctx, viewer, None)
        .await
        .map_err(DomainError::Invalid)
}

/// The options of choice param `param` of page `page_id`: what its
/// `options_from` lists for `supplied` (the page's current values), as
/// rows of `value`, `label` and an optional `group`.
pub async fn param_options(
    svc: &Services,
    page_id: &str,
    param: &str,
    supplied: &BTreeMap<String, SqlCell>,
    ctx: &ViewContext,
    viewer: &Actor,
) -> Result<SqlQueryResult, DomainError> {
    let page = svc
        .extension_catalog
        .find_page(&svc.layout.project_dir, page_id)
        .map_err(|_| DomainError::Invalid(format!("no page `{page_id}`")))?;
    let declared = page
        .params
        .iter()
        .chain(page.nodes().into_iter().flat_map(|n| match &n.kind {
            super::schema::NodeKind::Viz { config, .. } => config.params.iter(),
            _ => [].iter(),
        }));
    let from = declared
        .filter(|p| p.name == param)
        .find_map(|p| p.options_from.clone())
        .ok_or_else(|| {
            DomainError::Invalid(format!(
                "`{page_id}` has no param `{param}` with `options_from`"
            ))
        })?;
    let rows = source_rows(svc, &from, supplied, ctx, viewer).await?;
    if let Some(why) = Shape::Options.problem(&rows.columns) {
        return Err(DomainError::Invalid(format!("`{from}`: {why}")));
    }
    Ok(rows)
}
