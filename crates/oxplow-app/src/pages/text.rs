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
use crate::pages::spec::VizRun;

/// `run` as text.
pub async fn page_text(
    svc: &crate::Services,
    root: &Path,
    run: &PageRun,
) -> Result<String, DomainError> {
    // A page that is one viz is drawn bare, no heading of its own under the
    // page's: its title line names its node.
    let mut out = match root_viz(run) {
        Some(_) => format!("# {} [node {}]\n", run.page.title, run.tree.id),
        None => format!("# {}\n", run.page.title),
    };
    write_tree(svc, root, run, &run.tree, "", 2, &mut out).await?;
    if let Some(rail) = &run.rail {
        out.push_str("\n## Rail\n");
        write_tree(svc, root, run, rail, "", 3, &mut out).await?;
    }
    if !run.see_also.is_empty() {
        out.push_str("\n## See also\n\n");
        for link in &run.see_also {
            out.push_str(&format!("- [{}]({})\n", link.title, link.uri));
        }
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

/// The run's root node when it is a viz: the page is that one viz, drawn
/// bare (no heading under the page's own, or under the node that embeds
/// it), so the heading above it names this node.
fn root_viz(run: &PageRun) -> Option<&Node> {
    node(&run.page, &run.tree.id).filter(|n| matches!(n.kind, NodeKind::Viz { .. }))
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
        // What the section holds, as the person reads it from the ⓘ.
        let describe = |def: &Node, out: &mut String| {
            if let Some(d) = &def.description {
                out.push_str(d);
                out.push_str("\n\n");
            }
        };
        // An embedded run under `title`: its heading, then its tree. One
        // that is a single viz reads under that heading (named for the viz's
        // node), not under a second one.
        let embedded = |child: &'r PageRun, title: &str, def: Option<&Node>, out: &mut String| {
            let nested = format!("{id}/");
            match root_viz(child) {
                Some(_) => out.push_str(&format!(
                    "\n{hashes} {title} [node {nested}{}]\n\n",
                    child.tree.id
                )),
                None => out.push_str(&heading(title)),
            }
            if let Some(def) = def {
                describe(def, out);
            }
            nested
        };
        let Some(def) = node(&run.page, &at.id) else {
            // A page another extension mounted at a slot: its own run.
            if let Some(child) = &at.embed {
                let nested = embedded(child, &child.page.title, None, out);
                write_tree(svc, root, child, &child.tree, &nested, level + 1, out).await?;
            }
            return Ok(());
        };
        match &def.kind {
            NodeKind::Stack { .. } | NodeKind::Grid { .. } | NodeKind::Switch { .. } => {
                if let Some(t) = &def.title {
                    out.push_str(&heading(t));
                    describe(def, out);
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
                // The run's root viz reads under the heading already written
                // for the page or the node that embeds it.
                if !std::ptr::eq(at, &run.tree) {
                    out.push_str(&heading(def.title.as_deref().unwrap_or(&cfg.title)));
                }
                describe(def, out);
                let failed = from
                    .as_ref()
                    .and_then(|f| run.loaders.get(f))
                    .and_then(|l| l.error.clone());
                match (failed, node_run(run, def)) {
                    (Some(error), _) => out.push_str(&format!("({error})\n")),
                    (None, Some(viz_run)) => {
                        if let Some(lead) = lead_text(&viz_run) {
                            out.push_str(&format!("{lead}\n\n"));
                        }
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
                    let nested = embedded(child, title, Some(def), out);
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

/// The node's `lead` paragraph: its column's text in the first row, none
/// when null or empty.
fn lead_text(run: &VizRun) -> Option<String> {
    let column = run.config.lead.as_ref()?;
    let at = run.result.columns.iter().position(|c| c == column)?;
    match run.result.rows.first()?.get(at)? {
        oxplow_db::SqlCell::Text(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        _ => None,
    }
}
