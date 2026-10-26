//! A page run as text: what an agent reads of a page (`run_page`,
//! `get_open_page`, Copy, Add to Context) and what a person sees, in the
//! same structure. Each node reads under a heading that names its id
//! (`[node files]`; an embed's nodes `[node <embed>/<id>]`), so an agent
//! can point the person at one (`#node=<id>`). A viz node renders as its
//! viz would (`viz_text::render`, one renderer per viz, tables cut at
//! [`crate::viz_text::MAX_ROWS`]).

use std::path::Path;

use futures::future::BoxFuture;
use oxplow_domain::DomainError;

use super::runner::{PageRun, RunNode};
use super::schema::{Node, NodeKind, Page};
use crate::extensions::VizRun;

/// `run` as text.
pub async fn page_text(
    svc: &crate::Services,
    root: &Path,
    run: &PageRun,
) -> Result<String, DomainError> {
    let mut out = format!("# {}\n", run.page.title);
    write_tree(svc, root, run, &run.tree, "", 2, &mut out).await?;
    if let Some(rail) = &run.rail {
        out.push_str("\n## Rail\n");
        write_tree(svc, root, run, rail, "", 3, &mut out).await?;
    }
    Ok(out)
}

/// The viz run a viz node shows: its config, the page's params, its
/// loader's rows. `None` for a node that isn't a viz, or whose loader
/// didn't run.
pub fn node_run(run: &PageRun, node: &Node) -> Option<VizRun> {
    let NodeKind::Viz { from, config: cfg } = &node.kind else {
        return None;
    };
    let result = match from {
        Some(f) => run.loaders.get(f)?.result.clone(),
        None => oxplow_db::SqlQueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            reads: oxplow_db::Reads::default(),
            freshness: Vec::new(),
            generation: None,
            coverage: Vec::new(),
        },
    };
    Some(VizRun {
        config: cfg.as_ref().clone(),
        params: run.params.clone(),
        result,
    })
}

/// The page's node with `id`.
pub fn node<'p>(page: &'p Page, id: &str) -> Option<&'p Node> {
    page.nodes().into_iter().find(|n| n.id == id)
}

#[allow(clippy::too_many_arguments)]
fn write_tree<'r>(
    svc: &'r crate::Services,
    root: &'r Path,
    run: &'r PageRun,
    at: &'r RunNode,
    prefix: &'r str,
    level: usize,
    out: &'r mut String,
) -> BoxFuture<'r, Result<(), DomainError>> {
    Box::pin(async move {
        let id = format!("{prefix}{}", at.id);
        let hashes = "#".repeat(level.min(6));
        let heading = |title: &str| format!("\n{hashes} {title} [node {id}]\n\n");
        let Some(def) = node(&run.page, &at.id) else {
            // A page another extension mounted at a slot: its own run.
            if let Some(child) = &at.embed {
                out.push_str(&heading(&child.page.title));
                let nested = format!("{id}/");
                write_tree(svc, root, child, &child.tree, &nested, level + 1, out).await?;
            }
            return Ok(());
        };
        match &def.kind {
            NodeKind::Stack { .. } | NodeKind::Grid { .. } | NodeKind::Switch { .. } => {
                if let Some(t) = &def.title {
                    out.push_str(&heading(t));
                }
                let deeper = if def.title.is_some() {
                    level + 1
                } else {
                    level
                };
                for child in &at.children {
                    write_tree(svc, root, run, child, prefix, deeper, out).await?;
                }
            }
            NodeKind::Viz { from, config: cfg } => {
                out.push_str(&heading(def.title.as_deref().unwrap_or(&cfg.title)));
                let failed = from
                    .as_ref()
                    .and_then(|f| run.loaders.get(f))
                    .and_then(|l| l.error.clone());
                match (failed, node_run(run, def)) {
                    (Some(error), _) => out.push_str(&format!("({error})\n")),
                    (None, Some(viz_run)) => {
                        let resolved = crate::viz_text::resolve(svc, root, &viz_run).await?;
                        out.push_str(&crate::viz_text::render(&viz_run, &resolved));
                        out.push('\n');
                    }
                    (None, None) => out.push_str("(its loader didn't run)\n"),
                }
            }
            NodeKind::Embed { .. } => match (&at.embed, &at.error) {
                (Some(child), _) => {
                    let title = def.title.as_deref().unwrap_or(&child.page.title);
                    out.push_str(&heading(title));
                    let nested = format!("{id}/");
                    write_tree(svc, root, child, &child.tree, &nested, level + 1, out).await?;
                }
                (None, Some(e)) => {
                    out.push_str(&heading(def.title.as_deref().unwrap_or("Embedded page")));
                    out.push_str(&format!("({e})\n"));
                }
                (None, None) => {}
            },
            NodeKind::Markdown { text } => {
                if let Some(t) = &def.title {
                    out.push_str(&heading(t));
                }
                out.push_str(text);
                out.push('\n');
            }
            NodeKind::Component { component, .. } => {
                out.push_str(&heading(def.title.as_deref().unwrap_or(component)));
                out.push_str(&format!(
                    "(custom component `{component}`: it shows the page's loaders)\n"
                ));
            }
            NodeKind::Slot { .. } => {
                for child in &at.children {
                    write_tree(svc, root, run, child, prefix, level, out).await?;
                }
            }
        }
        Ok(())
    })
}
