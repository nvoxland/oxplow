//! Every lens has a text rendering (design rule 17,
//! `.context/extensions.md`): what an agent reads instead
//! of the rows, what `copy` puts on the clipboard, and what a panel says
//! when read as text. One renderer per kit component, each saying what
//! the component shows — a chart's series, a treemap's sizes, a grid's
//! children in order, a tree's nesting, a hunk's diff — capped at
//! [`MAX_ROWS`] rows.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{Reads, SqlCell};
use oxplow_domain::DomainError;
use serde::Serialize;

use crate::extensions::{AlertState, LensRun, LensViz};

/// Rows a table-like rendering shows before saying how many more there
/// are.
pub const MAX_ROWS: usize = 50;

/// Files a `hunks` rendering diffs, and the bytes of each file's diff.
pub const MAX_HUNK_FILES: usize = 20;
pub const MAX_HUNK_BYTES: usize = 8_000;

/// What a component needs beyond its rows: the runs of a document's or a
/// dashboard's lens parts (in part order) and of a dashboard's row tiles,
/// a
/// `hunks` lens's diffs (one per row, in row order, up to
/// [`MAX_HUNK_FILES`]) or a `diff` lens's one, a `code` lens's files.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// A composite's lens parts, run, each with what it needs in turn (a
    /// section that is a document has its own parts).
    pub children: Vec<(LensRun, Resolved)>,
    /// A dashboard's row tiles, one per row (up to [`MAX_ROWS`]): a lens or
    /// `sql` tile's run, or why it can't run; a text tile's none.
    pub row_tiles: Vec<Option<Result<LensRun, String>>>,
    pub diffs: Vec<String>,
    /// A `code` lens's files, one per row (up to [`MAX_HUNK_FILES`]): each
    /// one's text at its revision, cut to its marked lines.
    pub files: Vec<String>,
    /// An `approvals` lens's pending proposals, one line each.
    pub approvals: Vec<String>,
    /// An `answer` lens's answer, as showing it returned it.
    pub answer: Option<String>,
}

/// A lens run as an agent reads it: the text, and what it needs to know
/// about the run — not the rows, which the text carries.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensText {
    /// The lens id (`<extension>/<slug>`).
    pub lens: String,
    pub title: String,
    /// The parameter values used.
    pub params: BTreeMap<String, SqlCell>,
    pub columns: Vec<String>,
    pub row_count: usize,
    /// The query stopped at its row limit.
    pub truncated: bool,
    pub reads: Reads,
    pub alert: Option<AlertState>,
    pub text: String,
}

/// `run` as an agent reads it; what its component needs beyond the rows
/// is resolved first ([`resolve`]).
pub async fn text_run(
    svc: &crate::Services,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<LensText, DomainError> {
    let text = text_of(svc, root, run, ctx).await?;
    Ok(LensText {
        lens: run.lens.id.clone(),
        title: run.lens.title.clone(),
        params: run.params.clone(),
        columns: run.result.columns.clone(),
        row_count: run.result.rows.len(),
        truncated: run.result.truncated,
        reads: run.result.reads.clone(),
        alert: run.alert.clone(),
        text,
    })
}

/// `run`'s text, resolved first.
pub async fn text_of(
    svc: &crate::Services,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<String, DomainError> {
    Ok(render(run, &resolve(svc, root, run, ctx).await?))
}

/// What `run`'s component needs beyond its rows: a document's or a
/// dashboard's lens parts, each run with the params it declares from the
/// lens's, and a dashboard's row tiles; a `hunks` lens's diffs,
/// each file read at both revisions in `root`.
pub async fn resolve(
    svc: &crate::Services,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<Resolved, DomainError> {
    let mut out = Resolved::default();
    match run.lens.viz {
        LensViz::Document | LensViz::Dashboard => {
            let given = crate::extensions::part_params(run);
            let viewed = run.lens.viewed_parts(&run.params);
            for id in viewed.iter().filter_map(|p| p.lens.as_deref()) {
                let child = run_part(svc, root, id, &given, ctx).await?;
                let inner = Box::pin(resolve(svc, root, &child, ctx)).await?;
                out.children.push((child, inner));
            }
            if let Some(tile) = &run.lens.tile {
                for row in run.result.rows.iter().take(MAX_ROWS) {
                    out.row_tiles
                        .push(row_tile(svc, root, run, tile, row, &given, ctx).await);
                }
            }
        }
        LensViz::Approvals => out.approvals = pending_proposals(svc, run).await?,
        LensViz::Code => {
            let c = run.lens.code.clone().unwrap_or_default();
            let (Some(p), Some(r)) = (
                column(run, c.path.as_ref()),
                column(run, c.revision.as_ref()),
            ) else {
                return Ok(out);
            };
            let l = column(run, c.lines.as_ref());
            for row in run.result.rows.iter().take(MAX_HUNK_FILES) {
                let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
                let lines = l.and_then(|i| parse_lines(&at(i)));
                out.files
                    .push(file_text(svc, root, &at(p), &at(r), lines).await);
            }
        }
        LensViz::Answer => {
            let a = run.lens.answer.clone().unwrap_or_default();
            let id = column(run, a.uri.as_ref())
                .and_then(|i| run.result.rows.first()?.get(i).map(cell))
                .and_then(|uri| {
                    oxplow_domain::uri::build::parse_as::<i64>(
                        &uri,
                        oxplow_domain::uri::resources::ANSWER,
                    )
                    .ok()
                });
            out.answer = Some(match id {
                Some(id) => match Box::pin(crate::commands::lens::text_answer(svc, id)).await {
                    Ok((_, text)) => text,
                    Err(e) => format!("(that answer can't be read: {e})"),
                },
                None => "(no answer is named)".into(),
            });
        }
        LensViz::Editor => {
            let e = run.lens.editor.clone().unwrap_or_default();
            let (Some(p), Some(row)) = (column(run, e.path.as_ref()), run.result.rows.first())
            else {
                return Ok(out);
            };
            let path = row.get(p).map(cell).unwrap_or_default();
            out.files
                .push(file_text(svc, root, &path, "working", None).await);
        }
        LensViz::Diff => {
            let d = run.lens.diff.clone().unwrap_or_default();
            let (Some(p), Some(f), Some(t), Some(row)) = (
                column(run, d.path.as_ref()),
                column(run, d.from.as_ref()),
                column(run, d.to.as_ref()),
                run.result.rows.first(),
            ) else {
                return Ok(out);
            };
            let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
            out.diffs
                .push(file_diff(svc, root, &at(p), &at(f), &at(t)).await);
        }
        LensViz::Hunks => {
            let h = run.lens.hunks.clone().unwrap_or_default();
            let (Some(p), Some(f), Some(t)) = (
                column(run, h.path.as_ref()),
                column(run, h.from.as_ref()),
                column(run, h.to.as_ref()),
            ) else {
                return Ok(out);
            };
            for r in run.result.rows.iter().take(MAX_HUNK_FILES) {
                let (path, from, to) = (
                    r.get(p).map(cell).unwrap_or_default(),
                    r.get(f).map(cell).unwrap_or_default(),
                    r.get(t).map(cell).unwrap_or_default(),
                );
                out.diffs
                    .push(file_diff(svc, root, &path, &from, &to).await);
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Lens `id` run as a part: with the params it declares from `given`.
async fn run_part(
    svc: &crate::Services,
    root: &Path,
    id: &str,
    given: &BTreeMap<String, SqlCell>,
    ctx: &crate::extensions::LensContext,
) -> Result<LensRun, DomainError> {
    let child = svc.extension_catalog.find_lens(root, id)?;
    let params = given
        .iter()
        .filter(|(k, _)| child.params.iter().any(|p| &p.name == *k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    crate::extensions::run_lens(&svc.sql, &svc.extension_catalog, root, id, params, ctx).await
}

/// A dashboard row's tile run: its `lens` as a part, else its `sql` shown
/// as its `display` (a chart display by its `chart`; `metric`, or a chart
/// without one, as a table). A text tile has none; one that can't run
/// says why, and the others still run.
async fn row_tile(
    svc: &crate::Services,
    root: &Path,
    run: &LensRun,
    tile: &crate::extensions::LensTile,
    row: &[SqlCell],
    given: &BTreeMap<String, SqlCell>,
    ctx: &crate::extensions::LensContext,
) -> Option<Result<LensRun, String>> {
    let at = |role: &Option<String>| {
        column(run, role.as_ref())
            .and_then(|i| row.get(i))
            .map(cell)
            .filter(|v| !v.is_empty())
    };
    let ran = if let Some(id) = at(&tile.lens) {
        run_part(svc, root, &id, given, ctx).await
    } else {
        let sql = at(&tile.sql)?;
        let chart = at(&tile.chart)
            .and_then(|c| serde_json::from_str::<crate::extensions::LensChart>(&c).ok());
        let viz = match (at(&tile.display).as_deref(), &chart) {
            (Some("list"), _) => LensViz::List,
            (Some("number"), _) => LensViz::Number,
            (Some("markdown"), _) => LensViz::Markdown,
            (Some("bar"), Some(_)) => LensViz::Bar,
            (Some("line"), Some(_)) => LensViz::Line,
            (Some("treemap"), Some(_)) => LensViz::Treemap,
            _ => LensViz::Table,
        };
        let spec = crate::extensions::LensSpec {
            title: "Query".into(),
            description: String::new(),
            query: sql,
            viz,
            params: Vec::new(),
            columns: Vec::new(),
            empty: None,
            chart,
            tree: None,
            timeline: None,
            steps: None,
            hunks: None,
            code: None,
            diff: None,
            markdown: None,
            form: None,
            group: None,
            emphasis: None,
            depth: None,
        };
        let id = format!("{}#tile", run.lens.id);
        crate::extensions::run_spec(&svc.sql, &id, &spec, BTreeMap::new(), ctx).await
    };
    Some(ran.map_err(|e| e.to_string()))
}

/// The pending proposals an `approvals` lens shows, one line each: its
/// params `thread` (a thread ref) and `setting` (a config key) narrow them.
async fn pending_proposals(
    svc: &crate::Services,
    run: &LensRun,
) -> Result<Vec<String>, DomainError> {
    let param = |name: &str| run.params.get(name).map(cell).filter(|v| !v.is_empty());
    let result = svc
        .sql
        .query_sql(
            "SELECT uri, COALESCE(json_extract(preview, '$.summary'), command), actor_kind, \
                    (SELECT title FROM v_thread t WHERE t.id = p.thread_id) \
             FROM v_command_proposal p \
             WHERE decision = 'pending' \
               AND (?1 IS NULL OR thread_id = (SELECT id FROM v_thread WHERE uri = ?1)) \
               AND (?2 IS NULL OR key = ?2) \
             ORDER BY id DESC",
            vec![
                param("thread").map_or(SqlCell::Null(()), SqlCell::Text),
                param("setting")
                    .map(|key| oxplow_domain::uri::build::config_uri(&key))
                    .map_or(SqlCell::Null(()), SqlCell::Text),
            ],
            None,
        )
        .await?;
    Ok(result
        .rows
        .iter()
        .map(|r| {
            let at = |i: usize| r.get(i).map(cell).unwrap_or_default();
            let by = match at(3) {
                title if !title.is_empty() => format!("the {} in “{title}”", at(2)),
                _ => format!("a {}", at(2)),
            };
            format!("- {} — {} (proposed by {by})", at(0), at(1))
        })
        .collect())
}

/// Lines `"12-40"` (or `"12"`) as a 1-based inclusive range, ends in
/// order; none when the text isn't one.
pub fn parse_lines(text: &str) -> Option<(usize, usize)> {
    let (a, b) = match text.trim().split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (text.trim(), text.trim()),
    };
    let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
    (a > 0 && b > 0).then(|| (a.min(b), a.max(b)))
}

/// `path` at revision `rev`, cut to `lines` when given and to
/// [`MAX_HUNK_BYTES`], or why it can't be shown.
async fn file_text(
    svc: &crate::Services,
    root: &Path,
    path: &str,
    rev: &str,
    lines: Option<(usize, usize)>,
) -> String {
    let parsed: oxplow_domain::vcs::Revision = match rev.parse() {
        Ok(r) => r,
        Err(e) => return format!("(can't read it: {e})"),
    };
    let text = match svc.trees.read_at(root, &parsed, path).await {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => return format!("(not there at {rev})"),
        Err(e) => return format!("(can't read it: {e})"),
    };
    let text = match lines {
        Some((from, to)) => text
            .lines()
            .skip(from - 1)
            .take(to + 1 - from)
            .collect::<Vec<_>>()
            .join("\n"),
        None => text,
    };
    if text.len() <= MAX_HUNK_BYTES {
        return text;
    }
    let mut end = MAX_HUNK_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n(cut at {MAX_HUNK_BYTES} bytes)", &text[..end])
}

/// `path`'s unified diff from revision `from` to `to`, or why there is
/// none.
async fn file_diff(svc: &crate::Services, root: &Path, path: &str, from: &str, to: &str) -> String {
    let side = |rev: &str| {
        let rev = rev.to_string();
        async move {
            let parsed: oxplow_domain::vcs::Revision = rev.parse()?;
            let bytes = svc
                .trees
                .read_at(root, &parsed, path)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<String, String>(
                bytes
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default(),
            )
        }
    };
    match (side(from).await, side(to).await) {
        (Ok(old), Ok(new)) if old == new => "(no changes)".to_string(),
        (Ok(old), Ok(new)) => {
            crate::wiki_drift::unified_diff(
                &old,
                &new,
                &format!("{path}@{from}"),
                &format!("{path}@{to}"),
                MAX_HUNK_BYTES,
            )
            .0
        }
        (Err(e), _) | (_, Err(e)) => format!("(can't read it: {e})"),
    }
}

/// A view document's sections, the one in view in bold, and how to read
/// another; none for any other lens.
fn sections_line(run: &LensRun) -> Option<String> {
    let view = run.lens.view.as_deref()?;
    let param = run.lens.params.iter().find(|p| p.name == view)?;
    let shown: Vec<&str> = run
        .lens
        .viewed_parts(&run.params)
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    let names: Vec<String> = param
        .options
        .iter()
        .map(|o| {
            if shown.contains(&o.value.as_str()) {
                format!("**{}**", o.label)
            } else {
                o.label.clone()
            }
        })
        .collect();
    Some(format!(
        "Sections: {} — run with `{view}=<id>` ({}) for another.",
        names.join(", "),
        param
            .options
            .iter()
            .map(|o| o.value.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// `run` as text. A document or dashboard renders its parts in order
/// under their titles (a lens part its resolved run, a markdown part its
/// prose), then a dashboard's row tiles; a `hunks` lens its resolved
/// diffs; every other component renders its rows.
pub fn render(run: &LensRun, resolved: &Resolved) -> String {
    let rows = &run.result.rows;
    if matches!(run.lens.viz, LensViz::Document | LensViz::Dashboard) {
        let mut runs = resolved.children.iter();
        let parts = run
            .lens
            .viewed_parts(&run.params)
            .into_iter()
            .filter_map(|part| match (&part.lens, &part.markdown) {
                (Some(_), _) => runs.next().map(|(c, inner)| {
                    format!(
                        "### {}\n\n{}",
                        part.title.as_deref().unwrap_or(&c.lens.title),
                        render(c, inner)
                    )
                }),
                (None, Some(md)) => Some(match &part.title {
                    Some(t) => format!("### {t}\n\n{md}"),
                    None => md.clone(),
                }),
                (None, None) => None,
            });
        return sections_line(run)
            .into_iter()
            .chain(parts)
            .chain(row_tiles(run, resolved))
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if run.lens.viz == LensViz::Form {
        // An agent runs the command itself; the text says which.
        let f = run.lens.form.clone().unwrap_or_default();
        return format!(
            "A form that runs `{}`{}.",
            f.command.unwrap_or_default(),
            f.defaults
                .filter(|d| d.as_object().is_some_and(|o| !o.is_empty()))
                .map(|d| format!(", starting from {d}"))
                .unwrap_or_default()
        );
    }
    if rows.is_empty() {
        return run
            .lens
            .empty
            .clone()
            .unwrap_or_else(|| "No rows.".to_string());
    }
    let body = match run.lens.viz {
        LensViz::Markdown => {
            // Its `text` column (else the first) of the first row.
            let at = run
                .lens
                .markdown
                .as_ref()
                .and_then(|m| m.text.as_deref())
                .and_then(|t| run.result.columns.iter().position(|c| c == t))
                .unwrap_or(0);
            rows.first()
                .and_then(|r| r.get(at))
                .map(cell)
                .unwrap_or_default()
        }
        LensViz::Number => rows
            .first()
            .and_then(|r| r.first())
            .map(cell)
            .unwrap_or_default(),
        LensViz::Bar => bar(run),
        LensViz::Line => line(run),
        LensViz::Treemap => treemap(run),
        LensViz::Tree => tree(run),
        LensViz::Timeline => timeline(run),
        LensViz::Detail => detail(run),
        // A board reads as its columns, each a section of its cards.
        LensViz::Board => table(run),
        LensViz::Steps => steps(run),
        LensViz::Hunks => hunks(run, &resolved.diffs),
        LensViz::Code => code(run, &resolved.files),
        LensViz::Editor => editor(run, &resolved.files),
        LensViz::Answer => resolved.answer.clone().unwrap_or_else(|| table(run)),
        LensViz::Terminal => terminals(run),
        LensViz::Embed => embed(run),
        LensViz::Diff => one_diff(run, &resolved.diffs),
        LensViz::Approvals => {
            if resolved.approvals.is_empty() {
                "No proposals wait for a decision.".to_string()
            } else {
                format!(
                    "Proposals waiting for a person's decision (an agent can't decide them):\n{}",
                    resolved.approvals.join("\n")
                )
            }
        }
        LensViz::Table | LensViz::List | LensViz::Document | LensViz::Dashboard | LensViz::Form => {
            table(run)
        }
        // A component's rendering is its own; an agent reads its rows.
        LensViz::Custom => format!(
            "(custom component `{}/{}`; its table rendering)\n{}",
            run.lens.extension,
            run.lens
                .custom
                .as_ref()
                .and_then(|c| c.component.clone())
                .unwrap_or_default(),
            table(run)
        ),
    };
    if run.result.truncated {
        format!("{body}\n(the query stopped at its row limit)")
    } else {
        body
    }
}

fn cell(c: &SqlCell) -> String {
    match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.clone(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    }
}

fn number(c: &SqlCell) -> Option<f64> {
    match c {
        SqlCell::Int(i) => Some(*i as f64),
        SqlCell::Real(r) => Some(*r),
        SqlCell::Text(t) => t.parse().ok(),
        _ => None,
    }
}

fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// A markdown table of `header` and `rows`, capped at [`MAX_ROWS`].
fn markdown_table(header: &[String], rows: &[Vec<String>]) -> String {
    let esc = |t: &str| t.replace('|', "\\|").replace('\n', " ");
    let mut out = format!(
        "| {} |\n|{}\n",
        header
            .iter()
            .map(|h| esc(h))
            .collect::<Vec<_>>()
            .join(" | "),
        " --- |".repeat(header.len())
    );
    for r in rows.iter().take(MAX_ROWS) {
        out.push_str(&format!(
            "| {} |\n",
            r.iter().map(|c| esc(c)).collect::<Vec<_>>().join(" | ")
        ));
    }
    if rows.len() > MAX_ROWS {
        out.push_str(&format!("({} more rows)\n", rows.len() - MAX_ROWS));
    }
    out
}

/// What a table or list shows: the declared columns in their order (with
/// their labels), or every result column when none are declared; a
/// grouped one, a section per group in the order each first appears.
fn table(run: &LensRun) -> String {
    let shown = shown(run);
    let header: Vec<String> = shown.iter().map(|(_, l)| l.clone()).collect();
    let line = |r: &Vec<SqlCell>| -> Vec<String> {
        shown
            .iter()
            .map(|(i, _)| r.get(*i).map(cell).unwrap_or_default())
            .collect()
    };
    let by = run
        .lens
        .group
        .as_ref()
        .and_then(|g| column(run, Some(&g.by)));
    let Some(by) = by else {
        let rows: Vec<Vec<String>> = run.result.rows.iter().map(line).collect();
        return markdown_table(&header, &rows);
    };
    // The declared values first, in order (an empty one reads as none),
    // then the rest as each first appears.
    let mut groups: Vec<(String, Vec<Vec<String>>)> = run
        .lens
        .group
        .as_ref()
        .map(|g| g.values.iter().map(|v| (v.clone(), Vec::new())).collect())
        .unwrap_or_default();
    for r in &run.result.rows {
        let name = r.get(by).map(cell).unwrap_or_default();
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, rows)) => rows.push(line(r)),
            None => groups.push((name, vec![line(r)])),
        }
    }
    groups
        .iter()
        .map(|(name, rows)| match rows.is_empty() {
            true => format!("### {name}\n\n(none)\n"),
            false => format!("### {name}\n\n{}", markdown_table(&header, rows)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The displayed columns (the UI's `displayColumns`): the declared ones in
/// their order with their labels, else every result column — never the
/// row-styling ones (`group.by`, `emphasis`, `depth`).
fn shown(run: &LensRun) -> Vec<(usize, String)> {
    let styling: Vec<&String> = run
        .lens
        .group
        .iter()
        .map(|g| &g.by)
        .chain(run.lens.emphasis.iter())
        .chain(run.lens.depth.iter())
        .collect();
    let all: Vec<(usize, String)> = if run.lens.columns.is_empty() {
        run.result.columns.iter().cloned().enumerate().collect()
    } else {
        run.lens
            .columns
            .iter()
            .filter_map(|c| {
                let i = run.result.columns.iter().position(|k| k == &c.key)?;
                Some((i, c.label.clone().unwrap_or_else(|| c.key.clone())))
            })
            .collect()
    };
    all.into_iter()
        .filter(|(i, _)| !styling.contains(&&run.result.columns[*i]))
        .collect()
}

/// A tree: each row under the row its parent names, indented; rows whose
/// parent isn't in the result are roots.
fn tree(run: &LensRun) -> String {
    let t = run.lens.tree.clone().unwrap_or_default();
    let (Some(id), Some(parent), Some(label)) = (
        column(run, t.id.as_ref()),
        column(run, t.parent.as_ref()),
        column(run, t.label.as_ref()),
    ) else {
        return table(run);
    };
    // A lazy tree's run is one level: a row the `branch` column marks
    // opens by running the lens again with `lazy` set to its id.
    let lazy = t.lazy.as_deref().and_then(|p| {
        let branch = column(run, t.branch.as_ref())?;
        Some((p, branch))
    });
    let rows = &run.result.rows;
    let ids: Vec<String> = rows
        .iter()
        .map(|r| r.get(id).map(cell).unwrap_or_default())
        .collect();
    let parent_of: Vec<Option<usize>> = rows
        .iter()
        .map(|r| {
            let p = r.get(parent).map(cell).unwrap_or_default();
            (!p.is_empty())
                .then(|| ids.iter().position(|i| *i == p))
                .flatten()
        })
        .collect();
    // A row that opens lazily: marked `branch`, with no children here.
    let opens = |at: usize| -> bool {
        lazy.is_some_and(|(_, branch)| {
            rows[at].get(branch).is_some_and(truthy) && !parent_of.contains(&Some(at))
        })
    };
    struct Walk<'a> {
        rows: &'a [Vec<SqlCell>],
        label: usize,
        parent_of: &'a [Option<usize>],
        opens: &'a dyn Fn(usize) -> bool,
        seen: Vec<bool>,
        lines: Vec<String>,
    }
    impl Walk<'_> {
        fn walk(&mut self, at: usize, depth: usize) {
            if self.seen[at] {
                return;
            }
            self.seen[at] = true;
            self.lines.push(format!(
                "{}- {}{}",
                "  ".repeat(depth),
                if (self.opens)(at) { "▸ " } else { "" },
                self.rows[at].get(self.label).map(cell).unwrap_or_default()
            ));
            for child in 0..self.parent_of.len() {
                if self.parent_of[child] == Some(at) {
                    self.walk(child, depth + 1);
                }
            }
        }
    }
    let mut w = Walk {
        rows,
        label,
        parent_of: &parent_of,
        opens: &opens,
        seen: vec![false; rows.len()],
        lines: Vec::new(),
    };
    for root in (0..rows.len()).filter(|i| parent_of[*i].is_none()) {
        w.walk(root, 0);
    }
    let lines = w.lines;
    let mut text = capped_lines(lines);
    if let Some((param, _)) = lazy {
        let now = run.params.get(param).map(cell).unwrap_or_default();
        text.push_str(&format!(
            "\n\nA ▸ row opens with `{param}=<id>`: run the lens with it to read that level (this run: `{param}={now}`)."
        ));
    }
    text
}

/// A cell that counts as true: not null, zero, empty or false.
fn truthy(c: &SqlCell) -> bool {
    match c {
        SqlCell::Null(()) => false,
        SqlCell::Bool(b) => *b,
        SqlCell::Int(i) => *i != 0,
        SqlCell::Real(r) => *r != 0.0,
        SqlCell::Text(s) => !s.is_empty() && s != "0" && s != "false",
    }
}

/// A timeline: each row's time and label, oldest first, with its ref.
fn timeline(run: &LensRun) -> String {
    let t = run.lens.timeline.clone().unwrap_or_default();
    let (Some(at), Some(label)) = (column(run, t.at.as_ref()), column(run, t.label.as_ref()))
    else {
        return table(run);
    };
    let link = column(run, t.uri_column.as_ref());
    let mut entries: Vec<(String, String)> = run
        .result
        .rows
        .iter()
        .map(|r| {
            let mut line = format!(
                "{} — {}",
                r.get(at).map(cell).unwrap_or_default(),
                r.get(label).map(cell).unwrap_or_default()
            );
            if let Some(v) = link
                .and_then(|l| r.get(l))
                .map(cell)
                .filter(|v| !v.is_empty())
            {
                line.push_str(&format!(" ({v})"));
            }
            (r.get(at).map(cell).unwrap_or_default(), format!("- {line}"))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    capped_lines(entries.into_iter().map(|(_, l)| l).collect())
}

/// A detail: the first row's displayed columns as label/value lines.
fn detail(run: &LensRun) -> String {
    let Some(row) = run.result.rows.first() else {
        return String::new();
    };
    shown(run)
        .into_iter()
        .map(|(i, l)| format!("**{l}**: {}", row.get(i).map(cell).unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Steps: a numbered checklist — `[x]` done, `[>]` active, `[!]` failed,
/// `[ ]` anything else.
fn steps(run: &LensRun) -> String {
    let s = run.lens.steps.clone().unwrap_or_default();
    let Some(label) = column(run, s.label.as_ref()) else {
        return table(run);
    };
    let status = column(run, s.status.as_ref());
    capped_lines(
        run.result
            .rows
            .iter()
            .enumerate()
            .map(|(n, r)| {
                let mark = match status.and_then(|c| r.get(c)).map(cell).as_deref() {
                    Some("done") => "[x]",
                    Some("active") => "[>]",
                    Some("failed") => "[!]",
                    _ => "[ ]",
                };
                format!(
                    "{}. {mark} {}",
                    n + 1,
                    r.get(label).map(cell).unwrap_or_default()
                )
            })
            .collect(),
    )
}

/// Hunks: each row's file and revisions, then its diff.
fn hunks(run: &LensRun, diffs: &[String]) -> String {
    let h = run.lens.hunks.clone().unwrap_or_default();
    let (Some(p), Some(f), Some(t)) = (
        column(run, h.path.as_ref()),
        column(run, h.from.as_ref()),
        column(run, h.to.as_ref()),
    ) else {
        return table(run);
    };
    let rows = &run.result.rows;
    let mut out: Vec<String> = rows
        .iter()
        .zip(diffs)
        .map(|(r, diff)| {
            format!(
                "#### {} ({} → {})\n\n```diff\n{}\n```",
                r.get(p).map(cell).unwrap_or_default(),
                r.get(f).map(cell).unwrap_or_default(),
                r.get(t).map(cell).unwrap_or_default(),
                diff.trim_end()
            )
        })
        .collect();
    if rows.len() > diffs.len() {
        out.push(format!("({} more files)", rows.len() - diffs.len()));
    }
    out.join("\n\n")
}

/// Code: each row's file, revision and marked lines, then its text in a
/// fence named for its extension.
/// The terminals a lens shows, one line each. Their output isn't read here
/// (an agent never reads a person's terminal; a session's transcript is).
fn terminals(run: &LensRun) -> String {
    let t = run.lens.terminal.clone().unwrap_or_default();
    let Some(of) = column(run, t.of.as_ref()) else {
        return table(run);
    };
    let title = column(run, t.title.as_ref());
    let lines: Vec<String> = run
        .result
        .rows
        .iter()
        .map(|row| {
            let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
            let uri = at(of);
            let name = title.map(at).filter(|n| !n.is_empty()).unwrap_or_else(|| uri.clone());
            if uri.contains("/agent_session?") {
                format!("- Agent session {name} ({uri}): its output isn't readable here; read its transcript.")
            } else {
                format!("- Terminal {name} ({uri})")
            }
        })
        .collect();
    if lines.is_empty() {
        "No terminals.".into()
    } else {
        lines.join("\n")
    }
}

/// The file an editor shows, as saved: what an agent reads of it (the
/// person's unsaved edits stay in their window).
fn editor(run: &LensRun, files: &[String]) -> String {
    let e = run.lens.editor.clone().unwrap_or_default();
    let (Some(p), Some(row), Some(text)) = (
        column(run, e.path.as_ref()),
        run.result.rows.first(),
        files.first(),
    ) else {
        return table(run);
    };
    let path = row.get(p).map(cell).unwrap_or_default();
    format!(
        "#### {path}, being edited (as saved; unsaved edits aren't here)\n\n```{}\n{}\n```",
        fence_of(&path),
        text.trim_end()
    )
}

/// A web page: the first row's URL, as a link line (the page itself is
/// opened on the person's machine, never read here).
fn embed(run: &LensRun) -> String {
    let e = run.lens.embed.clone().unwrap_or_default();
    let (Some(u), Some(row)) = (column(run, e.url.as_ref()), run.result.rows.first()) else {
        return table(run);
    };
    let url = row.get(u).map(cell).unwrap_or_default();
    format!("[{url}]({url}) — a web page, opened in its own window")
}

/// A fence's language for `path`: its extension, when it is one word.
fn fence_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn code(run: &LensRun, files: &[String]) -> String {
    let c = run.lens.code.clone().unwrap_or_default();
    let (Some(p), Some(r)) = (
        column(run, c.path.as_ref()),
        column(run, c.revision.as_ref()),
    ) else {
        return table(run);
    };
    let l = column(run, c.lines.as_ref());
    let rows = &run.result.rows;
    let mut out: Vec<String> = rows
        .iter()
        .zip(files)
        .map(|(row, text)| {
            let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
            let path = at(p);
            let lines = l
                .and_then(|i| parse_lines(&at(i)))
                .map(|(a, b)| format!(":{a}-{b}"))
                .unwrap_or_default();
            let fence = fence_of(&path);
            format!(
                "#### {path}@{}{lines}\n\n```{fence}\n{}\n```",
                at(r),
                text.trim_end()
            )
        })
        .collect();
    if rows.len() > files.len() {
        out.push(format!("({} more files)", rows.len() - files.len()));
    }
    out.join("\n\n")
}

/// Diff: the first row's file and revisions, then its diff.
fn one_diff(run: &LensRun, diffs: &[String]) -> String {
    let d = run.lens.diff.clone().unwrap_or_default();
    let (Some(p), Some(f), Some(t), Some(row), Some(diff)) = (
        column(run, d.path.as_ref()),
        column(run, d.from.as_ref()),
        column(run, d.to.as_ref()),
        run.result.rows.first(),
        diffs.first(),
    ) else {
        return table(run);
    };
    let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
    format!(
        "#### {} ({} → {})\n\n```diff\n{}\n```",
        at(p),
        at(f),
        at(t),
        diff.trim_end()
    )
}

/// `lines` joined, capped at [`MAX_ROWS`] with how many more.
fn capped_lines(lines: Vec<String>) -> String {
    let more = lines.len().saturating_sub(MAX_ROWS);
    let mut out: Vec<String> = lines.into_iter().take(MAX_ROWS).collect();
    if more > 0 {
        out.push(format!("({more} more rows)"));
    }
    out.join("\n")
}

/// A dashboard's row tiles, each under its title (its `title` column,
/// else its run's): a lens or `sql` tile its resolved run, a text tile its
/// text (under no heading without a title).
fn row_tiles<'a>(run: &'a LensRun, resolved: &'a Resolved) -> impl Iterator<Item = String> + 'a {
    let tile = run.lens.tile.clone().unwrap_or_default();
    let (title, text) = (
        column(run, tile.title.as_ref()),
        column(run, tile.text.as_ref()),
    );
    run.result
        .rows
        .iter()
        .zip(&resolved.row_tiles)
        .filter_map(move |(row, ran)| {
            let at = |i: Option<usize>| {
                i.and_then(|i| row.get(i))
                    .map(cell)
                    .filter(|v| !v.is_empty())
            };
            let (heading, body) = match ran {
                Some(Ok(c)) => (
                    at(title).unwrap_or_else(|| c.lens.title.clone()),
                    render(c, &Resolved::default()),
                ),
                Some(Err(e)) => (
                    at(title).unwrap_or_else(|| "Query".into()),
                    format!("(can't run it: {e})"),
                ),
                None => match (at(title), at(text)) {
                    (Some(t), body) => (t, body.unwrap_or_default()),
                    (None, Some(body)) => return Some(body),
                    (None, None) => return None,
                },
            };
            Some(format!("### {heading}\n\n{body}").trim_end().to_string())
        })
}

fn column(run: &LensRun, name: Option<&String>) -> Option<usize> {
    let name = name?;
    run.result.columns.iter().position(|c| c == name)
}

/// Bars: each label and its value, and the total.
fn bar(run: &LensRun) -> String {
    let chart = run.lens.chart.clone().unwrap_or_default();
    let (Some(x), Some(y)) = (column(run, chart.x.as_ref()), column(run, chart.y.as_ref())) else {
        return table(run);
    };
    let mut total = 0.0;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for r in &run.result.rows {
        let v = r.get(y).and_then(number).unwrap_or(0.0);
        total += v;
        rows.push(vec![r.get(x).map(cell).unwrap_or_default(), fmt_number(v)]);
    }
    let header = vec![
        chart.x.clone().unwrap_or_default(),
        chart.y.clone().unwrap_or_default(),
    ];
    format!(
        "{}Total: {}",
        markdown_table(&header, &rows),
        fmt_number(total)
    )
}

/// Lines: one row per x, one column per series, and each series' total.
fn line(run: &LensRun) -> String {
    let chart = run.lens.chart.clone().unwrap_or_default();
    let (Some(x), Some(y)) = (column(run, chart.x.as_ref()), column(run, chart.y.as_ref())) else {
        return table(run);
    };
    let s = column(run, chart.series.as_ref());
    let mut series: Vec<String> = Vec::new();
    let mut xs: Vec<String> = Vec::new();
    let mut values: BTreeMap<(String, String), f64> = BTreeMap::new();
    for r in &run.result.rows {
        let xv = r.get(x).map(cell).unwrap_or_default();
        let name = s
            .and_then(|s| r.get(s))
            .map(cell)
            .unwrap_or_else(|| chart.y.clone().unwrap_or_default());
        let Some(v) = r.get(y).and_then(number) else {
            continue;
        };
        if !xs.contains(&xv) {
            xs.push(xv.clone());
        }
        if !series.contains(&name) {
            series.push(name.clone());
        }
        *values.entry((xv, name)).or_default() += v;
    }
    xs.sort();
    let mut header = vec![chart.x.clone().unwrap_or_default()];
    header.extend(series.iter().cloned());
    let rows: Vec<Vec<String>> = xs
        .iter()
        .map(|xv| {
            let mut row = vec![xv.clone()];
            row.extend(series.iter().map(|name| {
                values
                    .get(&(xv.clone(), name.clone()))
                    .map(|v| fmt_number(*v))
                    .unwrap_or_default()
            }));
            row
        })
        .collect();
    let totals: Vec<String> = series
        .iter()
        .map(|name| {
            let t: f64 = values
                .iter()
                .filter(|((_, n), _)| n == name)
                .map(|(_, v)| v)
                .sum();
            format!("{name} {}", fmt_number(t))
        })
        .collect();
    format!(
        "{}Totals: {}",
        markdown_table(&header, &rows),
        totals.join(", ")
    )
}

/// A treemap: each label's size (and group), largest first.
fn treemap(run: &LensRun) -> String {
    let chart = run.lens.chart.clone().unwrap_or_default();
    let (Some(label), Some(size)) = (
        column(run, chart.label.as_ref()),
        column(run, chart.size.as_ref()),
    ) else {
        return table(run);
    };
    let group = column(run, chart.group.as_ref());
    let mut items: Vec<(String, f64, Option<String>)> = run
        .result
        .rows
        .iter()
        .filter_map(|r| {
            let v = r.get(size).and_then(number)?;
            (v > 0.0).then(|| {
                (
                    r.get(label).map(cell).unwrap_or_default(),
                    v,
                    group.and_then(|g| r.get(g)).map(cell),
                )
            })
        })
        .collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut header = vec![
        chart.label.clone().unwrap_or_default(),
        chart.size.clone().unwrap_or_default(),
    ];
    if let Some(g) = &chart.group {
        header.push(g.clone());
    }
    let rows: Vec<Vec<String>> = items
        .into_iter()
        .map(|(l, v, g)| {
            let mut row = vec![l, fmt_number(v)];
            if group.is_some() {
                row.push(g.unwrap_or_default());
            }
            row
        })
        .collect();
    markdown_table(&header, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{Lens, LensChart};
    use oxplow_db::SqlQueryResult;

    /// A markdown lens reads as its `text` column, not its first.
    #[test]
    fn a_markdown_lens_reads_as_its_text_column() {
        let mut r = run(
            LensViz::Markdown,
            None,
            &["slug", "body"],
            vec![vec![t("a"), t("# Hello")]],
        );
        r.lens.markdown = Some(crate::extensions::LensMarkdown {
            text: Some("body".into()),
        });
        let out = render(&r, &Resolved::default());
        assert!(out.contains("# Hello") && !out.contains("\na\n"), "{out}");
    }

    fn run(
        viz: LensViz,
        chart: Option<LensChart>,
        columns: &[&str],
        rows: Vec<Vec<SqlCell>>,
    ) -> LensRun {
        LensRun {
            lens: Lens {
                id: "t/l".into(),
                extension: "t".into(),
                slug: "l".into(),
                title: "Lens".into(),
                description: String::new(),
                query: String::new(),
                viz,
                params: Vec::new(),
                columns: Vec::new(),
                empty: None,
                chart,
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
                hidden: false,
                actions: Vec::new(),
                alert: None,
                needs: Vec::new(),
                origin: Default::default(),
                tool: None,
            },
            params: BTreeMap::new(),
            result: SqlQueryResult {
                columns: columns.iter().map(|c| c.to_string()).collect(),
                rows,
                truncated: false,
                reads: Reads::default(),
                freshness: Vec::new(),
                generation: None,
                coverage: Vec::new(),
            },
            alert: None,
            warnings: Vec::new(),
            inactive: None,
        }
    }

    fn t(s: &str) -> SqlCell {
        SqlCell::Text(s.into())
    }

    fn chart(x: &str, y: &str, series: Option<&str>) -> Option<LensChart> {
        Some(LensChart {
            x: Some(x.into()),
            y: Some(y.into()),
            series: series.map(str::to_string),
            ..LensChart::default()
        })
    }

    /// A bar chart reads as its labels and values, with the total.
    #[test]
    fn a_bar_lens_is_a_series_table_with_its_total() {
        let r = run(
            LensViz::Bar,
            chart("day", "n", None),
            &["day", "n"],
            vec![
                vec![t("mon"), SqlCell::Int(3)],
                vec![t("tue"), SqlCell::Int(4)],
            ],
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "| day | n |\n| --- | --- |\n| mon | 3 |\n| tue | 4 |\nTotal: 7"
        );
    }

    /// Lines pivot to one column per series, each with its total.
    #[test]
    fn a_line_lens_has_a_column_per_series() {
        let r = run(
            LensViz::Line,
            chart("day", "n", Some("who")),
            &["day", "n", "who"],
            vec![
                vec![t("2026-09-02"), SqlCell::Int(1), t("a")],
                vec![t("2026-09-01"), SqlCell::Int(2), t("a")],
                vec![t("2026-09-01"), SqlCell::Real(0.5), t("b")],
            ],
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "| day | a | b |\n| --- | --- | --- |\n| 2026-09-01 | 2 | 0.5 |\n| 2026-09-02 | 1 |  |\nTotals: a 3, b 0.5"
        );
    }

    /// A treemap lists its labels largest first.
    #[test]
    fn a_treemap_is_its_labels_by_size() {
        let r = run(
            LensViz::Treemap,
            Some(LensChart {
                label: Some("path".into()),
                size: Some("lines".into()),
                ..LensChart::default()
            }),
            &["path", "lines"],
            vec![
                vec![t("a.rs"), SqlCell::Int(10)],
                vec![t("b.rs"), SqlCell::Int(30)],
                vec![t("c.rs"), SqlCell::Int(0)],
            ],
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "| path | lines |\n| --- | --- |\n| b.rs | 30 |\n| a.rs | 10 |\n"
        );
    }

    /// A table stops at MAX_ROWS and says how many more; an empty result
    /// says the lens's `empty` text.
    #[test]
    fn a_table_is_capped_and_an_empty_one_says_so() {
        let rows = (0..60).map(|i| vec![SqlCell::Int(i)]).collect();
        let text = render(
            &run(LensViz::Table, None, &["n"], rows),
            &Resolved::default(),
        );
        assert!(text.ends_with("(10 more rows)\n"), "{text}");
        assert!(text.contains("| 49 |") && !text.contains("| 50 |"));
        let mut empty = run(LensViz::Table, None, &["n"], Vec::new());
        assert_eq!(render(&empty, &Resolved::default()), "No rows.");
        empty.lens.empty = Some("Nothing waiting.".into());
        assert_eq!(render(&empty, &Resolved::default()), "Nothing waiting.");
    }

    /// A document reads as its parts, in order, under their titles: a
    /// lens part its run (a part's own title wins), a markdown part its
    /// prose.
    #[test]
    fn a_document_renders_its_parts_in_order() {
        let part = |id: &str, title: Option<&str>, lens: Option<&str>, markdown: Option<&str>| {
            crate::extensions::LensPart {
                id: id.into(),
                title: title.map(Into::into),
                lens: lens.map(Into::into),
                markdown: markdown.map(Into::into),
                size: None,
                replaceable: None,
                from: None,
            }
        };
        let grid = with(run(LensViz::Document, None, &[], Vec::new()), |l| {
            l.parts = vec![
                part("open", None, Some("x/open"), None),
                part("intro", Some("About"), None, Some("Read me.")),
                part("note", Some("Notes"), Some("x/note"), None),
            ]
        });
        let mut a = run(LensViz::Number, None, &["n"], vec![vec![SqlCell::Int(5)]]);
        a.lens.title = "Open".into();
        let mut b = run(LensViz::Markdown, None, &["m"], vec![vec![t("**ok**")]]);
        b.lens.title = "Note".into();
        assert_eq!(
            render(
                &grid,
                &Resolved {
                    children: vec![(a, Resolved::default()), (b, Resolved::default())],
                    diffs: Vec::new(),
                    files: Vec::new(),
                    approvals: Vec::new(),
                    row_tiles: Vec::new(),
                    answer: None,
                }
            ),
            "### Open\n\n5\n\n### About\n\nRead me.\n\n### Notes\n\n**ok**"
        );
    }

    fn lens_part(id: &str, title: Option<&str>, lens: &str) -> crate::extensions::LensPart {
        crate::extensions::LensPart {
            id: id.into(),
            title: title.map(Into::into),
            lens: Some(lens.into()),
            markdown: None,
            size: None,
            replaceable: None,
            from: None,
        }
    }

    fn number(title: &str, n: i64) -> LensRun {
        let mut r = run(LensViz::Number, None, &["n"], vec![vec![SqlCell::Int(n)]]);
        r.lens.title = title.into();
        r
    }

    /// A section that is a document reads as its own sections.
    #[test]
    fn a_nested_document_renders_its_own_sections() {
        let outer = with(run(LensViz::Document, None, &[], Vec::new()), |l| {
            l.parts = vec![lens_part("inner", Some("Agents"), "x/inner")]
        });
        let inner = with(run(LensViz::Document, None, &[], Vec::new()), |l| {
            l.parts = vec![
                lens_part("list", None, "x/list"),
                lens_part("prompt", None, "x/prompt"),
            ]
        });
        let inner_resolved = Resolved {
            children: vec![
                (number("List", 2), Resolved::default()),
                (number("Prompt", 3), Resolved::default()),
            ],
            ..Resolved::default()
        };
        assert_eq!(
            render(
                &outer,
                &Resolved {
                    children: vec![(inner, inner_resolved)],
                    ..Resolved::default()
                }
            ),
            "### Agents\n\n### List\n\n2\n\n### Prompt\n\n3"
        );
    }

    /// A view document reads as the section in view, under a line that
    /// names every section and how to read another.
    #[test]
    fn a_view_document_renders_only_the_viewed_section_naming_the_others() {
        let mut doc = with(run(LensViz::Document, None, &[], Vec::new()), |l| {
            l.parts = vec![
                lens_part("overview", Some("Every Setting"), "x/overview"),
                lens_part("agents", Some("Agents"), "x/agents"),
            ];
            l.view = Some("area".into());
            l.params = vec![crate::extensions::LensParam {
                name: "area".into(),
                default: Some(t("overview")),
                options: vec![
                    crate::extensions::LensParamOption {
                        value: "overview".into(),
                        label: "Every Setting".into(),
                    },
                    crate::extensions::LensParamOption {
                        value: "agents".into(),
                        label: "Agents".into(),
                    },
                ],
                ..Default::default()
            }];
        });
        doc.params.insert("area".into(), t("agents"));
        assert_eq!(
            render(
                &doc,
                &Resolved {
                    children: vec![(number("Agents", 4), Resolved::default())],
                    ..Resolved::default()
                }
            ),
            "Sections: Every Setting, **Agents** — run with `area=<id>` (overview, agents) for another.\n\n### Agents\n\n4"
        );
    }

    fn tiled(columns: &[&str], rows: Vec<Vec<SqlCell>>) -> LensRun {
        with(run(LensViz::Dashboard, None, columns, rows), |l| {
            l.tile = Some(crate::extensions::LensTile {
                id: Some("id".into()),
                title: Some("title".into()),
                lens: Some("lens".into()),
                sql: Some("sql".into()),
                display: Some("display".into()),
                text: Some("text".into()),
                ..Default::default()
            })
        })
    }

    /// A dashboard whose rows are its tiles reads as a section per row
    /// after its static parts: under the row's title, else its run's; a
    /// text tile its text, a tile that can't run why.
    #[test]
    fn a_row_tile_dashboard_reads_as_a_section_per_row() {
        let n = SqlCell::Null(());
        let mut grid = tiled(
            &["id", "title", "lens", "sql", "display", "text"],
            vec![
                vec![
                    t("dti1"),
                    n.clone(),
                    t("x/open"),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                ],
                vec![
                    t("dti2"),
                    t("Plan"),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                    t("Ship it."),
                ],
                vec![
                    t("dti3"),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                    t("Just text."),
                ],
                vec![
                    t("dti4"),
                    t("Bad"),
                    n.clone(),
                    t("SELEC"),
                    n.clone(),
                    n.clone(),
                ],
            ],
        );
        grid.lens.parts = vec![crate::extensions::LensPart {
            id: "intro".into(),
            title: Some("About".into()),
            lens: None,
            markdown: Some("Read me.".into()),
            size: None,
            replaceable: None,
            from: None,
        }];
        let mut open = run(LensViz::Number, None, &["n"], vec![vec![SqlCell::Int(5)]]);
        open.lens.title = "Open".into();
        let resolved = Resolved {
            row_tiles: vec![
                Some(Ok(open)),
                None,
                None,
                Some(Err("no such column".into())),
            ],
            ..Resolved::default()
        };
        assert_eq!(
            render(&grid, &resolved),
            "### About\n\nRead me.\n\n### Open\n\n5\n\n### Plan\n\nShip it.\n\nJust text.\n\n### Bad\n\n(can't run it: no such column)"
        );
    }

    /// A row tile's `sql` runs as its display; a lens that isn't there, or
    /// SQL that fails, says why and the others still run; a text tile
    /// runs nothing.
    #[tokio::test]
    async fn row_tiles_resolve_their_lens_and_sql() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let n = SqlCell::Null(());
        let grid = tiled(
            &["id", "title", "lens", "sql", "display", "text"],
            vec![
                vec![
                    t("dti1"),
                    n.clone(),
                    n.clone(),
                    t("SELECT 42 AS answer"),
                    t("number"),
                    n.clone(),
                ],
                vec![
                    t("dti2"),
                    n.clone(),
                    t("x/missing"),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                ],
                vec![
                    t("dti3"),
                    n.clone(),
                    n.clone(),
                    t("SELECT nope FROM nowhere"),
                    n.clone(),
                    n.clone(),
                ],
                vec![
                    t("dti4"),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                    n.clone(),
                    t("hi"),
                ],
            ],
        );
        let ctx = crate::extensions::LensContext::default();
        let resolved = resolve(&fx.svc, &root, &grid, &ctx).await.unwrap();
        assert_eq!(resolved.row_tiles.len(), 4);
        let answer = resolved.row_tiles[0].as_ref().unwrap().as_ref().unwrap();
        assert_eq!(answer.lens.viz, LensViz::Number);
        assert!(matches!(resolved.row_tiles[1], Some(Err(_))));
        assert!(matches!(resolved.row_tiles[2], Some(Err(_))));
        assert!(resolved.row_tiles[3].is_none());
        let text = render(&grid, &resolved);
        assert!(text.starts_with("### Query\n\n42\n\n"), "{text}");
    }

    fn with<F: FnOnce(&mut Lens)>(mut r: LensRun, f: F) -> LensRun {
        f(&mut r.lens);
        r
    }

    /// A grouped list reads as a section per group, in the order
    /// each first appears, without the group, emphasis or depth columns.
    #[test]
    fn a_grouped_list_is_a_section_per_group() {
        let rows = vec![
            vec![t("Ready"), t("a"), SqlCell::Int(0)],
            vec![t("Done"), t("b"), SqlCell::Int(1)],
            vec![t("Ready"), t("c"), SqlCell::Int(0)],
        ];
        let r = with(
            run(LensViz::List, None, &["bucket", "title", "d"], rows),
            |l| {
                l.group = Some(crate::extensions::LensGroup {
                    by: "bucket".into(),
                    link: None,
                    values: Vec::new(),
                });
                l.depth = Some("d".into());
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "### Ready\n\n| title |\n| --- |\n| a |\n| c |\n\n### Done\n\n| title |\n| --- |\n| b |\n"
        );
    }

    /// A board reads as its columns in `values` order, an empty one as
    /// none, then the rest as each first appears.
    #[test]
    fn a_board_reads_as_its_columns_in_order() {
        let rows = vec![vec![t("done"), t("a")], vec![t("later"), t("b")]];
        let r = with(run(LensViz::Board, None, &["state", "title"], rows), |l| {
            l.group = Some(crate::extensions::LensGroup {
                by: "state".into(),
                link: None,
                values: vec!["todo".into(), "done".into()],
            });
        });
        assert_eq!(
            render(&r, &Resolved::default()),
            "### todo\n\n(none)\n\n### done\n\n| title |\n| --- |\n| a |\n\n### later\n\n| title |\n| --- |\n| b |\n"
        );
    }

    /// A tree nests each row under the row its parent names.
    #[test]
    fn a_tree_nests_rows_under_their_parents() {
        let r = with(
            run(
                LensViz::Tree,
                None,
                &["id", "parent", "name"],
                vec![
                    vec![t("b"), t("a"), t("child")],
                    vec![t("a"), SqlCell::Null(()), t("root")],
                    vec![t("c"), t("b"), t("grandchild")],
                    vec![t("d"), t("gone"), t("orphan")],
                ],
            ),
            |l| {
                l.tree = Some(crate::extensions::LensTree {
                    id: Some("id".into()),
                    parent: Some("parent".into()),
                    label: Some("name".into()),
                    ..Default::default()
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- root\n  - child\n    - grandchild\n- orphan"
        );
    }

    /// A lazy tree's run is one level: its branches (rows that open) are
    /// marked, and the agent is told how to read one.
    #[test]
    fn a_lazy_tree_reads_as_one_level_with_its_opening_hint() {
        let mut r = with(
            run(
                LensViz::Tree,
                None,
                &["id", "parent", "name", "is_dir"],
                vec![
                    vec![t("src"), t(""), t("src"), SqlCell::Int(1)],
                    vec![t("a.rs"), t(""), t("a.rs"), SqlCell::Int(0)],
                ],
            ),
            |l| {
                l.tree = Some(crate::extensions::LensTree {
                    id: Some("id".into()),
                    parent: Some("parent".into()),
                    label: Some("name".into()),
                    lazy: Some("folder".into()),
                    branch: Some("is_dir".into()),
                })
            },
        );
        r.params.insert("folder".into(), t(""));
        assert_eq!(
            render(&r, &Resolved::default()),
            "- ▸ src\n- a.rs\n\nA ▸ row opens with `folder=<id>`: run the lens with it to read that level (this run: `folder=`)."
        );
    }

    /// A timeline lists its rows oldest first, with their refs.
    #[test]
    fn a_timeline_is_in_time_order() {
        let r = with(
            run(
                LensViz::Timeline,
                None,
                &["at", "what", "uri"],
                vec![
                    vec![
                        t("2026-09-30T10:00"),
                        t("shipped"),
                        t("oxplow:oxplow/commit?sha=abc"),
                    ],
                    vec![t("2026-09-29T09:00"), t("started"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.timeline = Some(crate::extensions::LensTimeline {
                    at: Some("at".into()),
                    label: Some("what".into()),
                    uri_column: Some("uri".into()),
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- 2026-09-29T09:00 — started\n- 2026-09-30T10:00 — shipped (oxplow:oxplow/commit?sha=abc)"
        );
    }

    /// A detail is its first row as label/value lines.
    #[test]
    fn a_detail_is_its_first_row() {
        let r = run(
            LensViz::Detail,
            None,
            &["title", "state"],
            vec![vec![t("Fix it"), t("done")], vec![t("ignored"), t("x")]],
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "**title**: Fix it\n**state**: done"
        );
    }

    /// Steps are a numbered checklist marked by status.
    #[test]
    fn steps_are_a_checklist() {
        let r = with(
            run(
                LensViz::Steps,
                None,
                &["step", "status"],
                vec![
                    vec![t("plan"), t("done")],
                    vec![t("build"), t("active")],
                    vec![t("test"), t("failed")],
                    vec![t("ship"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.steps = Some(crate::extensions::LensSteps {
                    label: Some("step".into()),
                    status: Some("status".into()),
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "1. [x] plan\n2. [>] build\n3. [!] test\n4. [ ] ship"
        );
    }

    /// Hunks are each row's file and revisions, then its diff; files past
    /// the resolved ones are counted.
    #[test]
    fn hunks_show_each_files_diff() {
        let r = with(
            run(
                LensViz::Hunks,
                None,
                &["path", "a", "b"],
                vec![
                    vec![t("x.rs"), t("git:abc"), t("working")],
                    vec![t("y.rs"), t("git:abc"), t("working")],
                ],
            ),
            |l| {
                l.hunks = Some(crate::extensions::LensHunks {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                })
            },
        );
        let resolved = Resolved {
            diffs: vec!["-old\n+new\n".into()],
            ..Resolved::default()
        };
        assert_eq!(
            render(&r, &resolved),
            "#### x.rs (git:abc → working)\n\n```diff\n-old\n+new\n```\n\n(1 more files)"
        );
    }

    /// A hunks lens resolves each row's diff from the workspace.
    #[tokio::test]
    async fn hunks_resolve_from_the_workspace() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("h.txt"), "one\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "h");
        std::fs::write(root.join("h.txt"), "two\n").unwrap();
        let r = with(
            run(
                LensViz::Hunks,
                None,
                &["path", "a", "b"],
                vec![vec![t("h.txt"), t(&format!("git:{sha}")), t("working")]],
            ),
            |l| {
                l.hunks = Some(crate::extensions::LensHunks {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                })
            },
        );
        let text = text_of(
            &fx.svc,
            &root,
            &r,
            &crate::extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert!(text.contains("-one") && text.contains("+two"), "{text}");
    }

    /// Code is each row's file, revision and marked lines, then its text
    /// in a fence named for its extension; files past the resolved ones
    /// are counted.
    #[test]
    fn code_shows_each_files_lines() {
        let r = with(
            run(
                LensViz::Code,
                None,
                &["path", "rev", "lines"],
                vec![
                    vec![t("src/x.rs"), t("git:abc"), t("8-3")],
                    vec![t("y.rs"), t("working"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.code = Some(crate::extensions::LensCode {
                    path: Some("path".into()),
                    revision: Some("rev".into()),
                    lines: Some("lines".into()),
                    sync: false,
                })
            },
        );
        let resolved = Resolved {
            files: vec!["fn x() {}\n".into()],
            ..Resolved::default()
        };
        assert_eq!(
            render(&r, &resolved),
            "#### src/x.rs@git:abc:3-8\n\n```rs\nfn x() {}\n```\n\n(1 more files)"
        );
        assert_eq!(parse_lines("12"), Some((12, 12)));
        assert_eq!(parse_lines(" 40-12 "), Some((12, 40)));
        assert_eq!(parse_lines("0-3"), None);
        assert_eq!(parse_lines("x"), None);
    }

    /// A diff is the first row's file and revisions, then its diff.
    #[test]
    fn a_diff_shows_the_first_rows_diff() {
        let r = with(
            run(
                LensViz::Diff,
                None,
                &["path", "a", "b"],
                vec![
                    vec![t("x.rs"), t("git:abc"), t("working")],
                    vec![t("y.rs"), t("git:abc"), t("working")],
                ],
            ),
            |l| {
                l.diff = Some(crate::extensions::LensDiff {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                    lines: None,
                    base: None,
                })
            },
        );
        let resolved = Resolved {
            diffs: vec!["-old\n+new\n".into()],
            ..Resolved::default()
        };
        assert_eq!(
            render(&r, &resolved),
            "#### x.rs (git:abc → working)\n\n```diff\n-old\n+new\n```"
        );
    }

    /// A code lens reads each row's file at its revision, cut to its
    /// lines; a diff lens diffs its first row's file.
    #[tokio::test]
    async fn code_and_diff_resolve_from_the_workspace() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("c.txt"), "one\ntwo\nthree\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "c");
        std::fs::write(root.join("c.txt"), "uno\n").unwrap();
        let ctx = crate::extensions::LensContext::default();
        let code = with(
            run(
                LensViz::Code,
                None,
                &["path", "rev", "lines"],
                vec![
                    vec![t("c.txt"), t(&format!("git:{sha}")), t("2-3")],
                    vec![t("gone.txt"), t("working"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.code = Some(crate::extensions::LensCode {
                    path: Some("path".into()),
                    revision: Some("rev".into()),
                    lines: Some("lines".into()),
                    sync: false,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &code, &ctx).await.unwrap();
        assert!(text.contains("```txt\ntwo\nthree\n```"), "{text}");
        assert!(text.contains("(not there at working)"), "{text}");
        let diff = with(
            run(
                LensViz::Diff,
                None,
                &["path", "a", "b"],
                vec![vec![t("c.txt"), t(&format!("git:{sha}")), t("working")]],
            ),
            |l| {
                l.diff = Some(crate::extensions::LensDiff {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                    lines: None,
                    base: None,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &diff, &ctx).await.unwrap();
        assert!(text.contains("-one") && text.contains("+uno"), "{text}");
    }

    /// An editor lens reads its first row's file as the working tree has
    /// it, and says the person's unsaved edits aren't in it.
    #[tokio::test]
    async fn an_editor_reads_the_working_file() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("e.txt"), "saved text\n").unwrap();
        let ctx = crate::extensions::LensContext::default();
        let editor = with(
            run(LensViz::Editor, None, &["path"], vec![vec![t("e.txt")]]),
            |l| {
                l.editor = Some(crate::extensions::LensEditor {
                    path: Some("path".into()),
                    lines: None,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &editor, &ctx).await.unwrap();
        assert!(
            text.contains("#### e.txt, being edited (as saved; unsaved edits aren't here)\n\n```txt\nsaved text\n```"),
            "{text}"
        );
    }

    /// A terminal lens names its terminals and never their output.
    #[test]
    fn terminals_read_as_their_names() {
        let r = with(
            run(
                LensViz::Terminal,
                None,
                &["uri", "title"],
                vec![
                    vec![t("oxplow:oxplow/terminal?id=trm1"), t("Build")],
                    vec![t("oxplow:oxplow/agent_session?id=ses2"), t("Claude")],
                ],
            ),
            |l| {
                l.terminal = Some(crate::extensions::LensTerminal {
                    of: Some("uri".into()),
                    title: Some("title".into()),
                    stopped: None,
                    conversation: None,
                    thread: None,
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- Terminal Build (oxplow:oxplow/terminal?id=trm1)\n- Agent session Claude (oxplow:oxplow/agent_session?id=ses2): its output isn't readable here; read its transcript."
        );
    }

    /// An embed lens reads as a link to its page.
    #[tokio::test]
    async fn an_embed_reads_as_a_link() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let ctx = crate::extensions::LensContext::default();
        let embed = with(
            run(
                LensViz::Embed,
                None,
                &["url"],
                vec![vec![t("https://example.com/a")]],
            ),
            |l| {
                l.embed = Some(crate::extensions::LensEmbed {
                    url: Some("url".into()),
                })
            },
        );
        let text = text_of(&fx.svc, &root, &embed, &ctx).await.unwrap();
        assert!(
            text.contains("[https://example.com/a](https://example.com/a)"),
            "{text}"
        );
    }

    /// An `answer` lens reads as its answer: the text showing it
    /// returned, so an agent reading the answer's page reads the answer.
    #[tokio::test]
    async fn an_answer_lens_reads_as_its_answer() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let agent = oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                "oxplow.lens.show",
                serde_json::json!({ "spec": { "title": "Busy Tasks", "query": "SELECT title FROM v_work_item", "viz": "table" } }),
                false,
            )
            .await
            .unwrap();
        let id = out.result["answer"].as_i64().unwrap();
        let answer = with(
            run(
                LensViz::Answer,
                None,
                &["uri"],
                vec![vec![t(&format!("oxplow:oxplow/answer?id={id}"))]],
            ),
            |l| {
                l.answer = Some(crate::extensions::LensAnswer {
                    uri: Some("uri".into()),
                    kept: None,
                })
            },
        );
        let root = fx.svc.layout.project_dir.clone();
        let ctx = crate::extensions::LensContext::default();
        let text = text_of(&fx.svc, &root, &answer, &ctx).await.unwrap();
        let shown = out.result["text"].as_str().unwrap();
        assert!(text.contains(shown), "{text}\n---\n{shown}");
    }

    /// An approvals lens reads the pending proposals itself: an agent
    /// sees what waits (and that it can't decide), narrowed by the
    /// lens's `thread` param; with none waiting it says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn approvals_list_the_pending_proposals() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let approvals = |params: &[(&str, SqlCell)]| {
            let mut r = run(
                LensViz::Approvals,
                None,
                &["n"],
                vec![vec![SqlCell::Int(1)]],
            );
            for (k, v) in params {
                r.params.insert(k.to_string(), v.clone());
            }
            r
        };
        let ctx = crate::extensions::LensContext::default();
        let none = text_of(&fx.svc, &root, &approvals(&[]), &ctx)
            .await
            .unwrap();
        assert_eq!(none, "No proposals wait for a decision.");

        // An effect's run that asks waits for a person.
        let effect = oxplow_domain::Actor::Effect {
            effect: "acme/notify".into(),
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(
                &effect,
                "oxplow.lsp.install_server",
                serde_json::json!({ "package": "rust-analyzer" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, oxplow_domain::CommandError::Proposed { .. }),
            "{err:?}"
        );
        let all = text_of(&fx.svc, &root, &approvals(&[]), &ctx)
            .await
            .unwrap();
        assert!(
            all.contains("waiting for a person's decision")
                && all.contains("oxplow:oxplow/proposal?id="),
            "{all}"
        );
        let other = text_of(
            &fx.svc,
            &root,
            &approvals(&[("thread", t("oxplow:oxplow/thread?id=thr999"))]),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(other, "No proposals wait for a decision.");
    }
}
