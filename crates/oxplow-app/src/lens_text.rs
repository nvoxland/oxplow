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

/// What a component needs beyond its rows: a grid's children's runs, a
/// `hunks` lens's diffs (one per row, in row order, up to
/// [`MAX_HUNK_FILES`]).
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub children: Vec<LensRun>,
    pub diffs: Vec<String>,
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

/// What `run`'s component needs beyond its rows: a grid's children, run
/// with the params each declares from the grid's; a `hunks` lens's diffs,
/// each file read at both revisions in `root`.
pub async fn resolve(
    svc: &crate::Services,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<Resolved, DomainError> {
    let mut out = Resolved::default();
    match run.lens.viz {
        LensViz::Grid => {
            for id in &run.lens.children {
                let child = svc.extension_catalog.find_lens(root, id)?;
                let params = run
                    .params
                    .iter()
                    .filter(|(k, _)| child.params.iter().any(|p| &p.name == *k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                out.children.push(
                    crate::extensions::run_lens(
                        &svc.sql,
                        &svc.extension_catalog,
                        root,
                        id,
                        params,
                        ctx,
                    )
                    .await?,
                );
            }
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

/// `run` as text. A grid renders its resolved children in order under
/// their titles, a `hunks` lens its resolved diffs; every other component
/// renders its rows.
pub fn render(run: &LensRun, resolved: &Resolved) -> String {
    let rows = &run.result.rows;
    if run.lens.viz == LensViz::Grid {
        return resolved
            .children
            .iter()
            .map(|c| {
                format!(
                    "### {}\n\n{}",
                    c.lens.title,
                    render(c, &Resolved::default())
                )
            })
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
        LensViz::Markdown | LensViz::Number => rows
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
        LensViz::Steps => steps(run),
        LensViz::Hunks => hunks(run, &resolved.diffs),
        LensViz::Table | LensViz::List | LensViz::Grid | LensViz::Form => table(run),
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
    let mut groups: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    for r in &run.result.rows {
        let name = r.get(by).map(cell).unwrap_or_default();
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, rows)) => rows.push(line(r)),
            None => groups.push((name, vec![line(r)])),
        }
    }
    groups
        .iter()
        .map(|(name, rows)| format!("### {name}\n\n{}", markdown_table(&header, rows)))
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
    let mut lines: Vec<String> = Vec::new();
    let mut seen = vec![false; rows.len()];
    fn walk(
        at: usize,
        depth: usize,
        rows: &[Vec<SqlCell>],
        label: usize,
        parent_of: &[Option<usize>],
        seen: &mut [bool],
        lines: &mut Vec<String>,
    ) {
        if seen[at] {
            return;
        }
        seen[at] = true;
        lines.push(format!(
            "{}- {}",
            "  ".repeat(depth),
            rows[at].get(label).map(cell).unwrap_or_default()
        ));
        for (child, p) in parent_of.iter().enumerate() {
            if *p == Some(at) {
                walk(child, depth + 1, rows, label, parent_of, seen, lines);
            }
        }
    }
    for root in (0..rows.len()).filter(|i| parent_of[*i].is_none()) {
        walk(root, 0, rows, label, &parent_of, &mut seen, &mut lines);
    }
    capped_lines(lines)
}

/// A timeline: each row's time and label, oldest first, with its ref.
fn timeline(run: &LensRun) -> String {
    let t = run.lens.timeline.clone().unwrap_or_default();
    let (Some(at), Some(label)) = (column(run, t.at.as_ref()), column(run, t.label.as_ref()))
    else {
        return table(run);
    };
    let link = column(run, t.ref_column.as_ref());
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

/// `lines` joined, capped at [`MAX_ROWS`] with how many more.
fn capped_lines(lines: Vec<String>) -> String {
    let more = lines.len().saturating_sub(MAX_ROWS);
    let mut out: Vec<String> = lines.into_iter().take(MAX_ROWS).collect();
    if more > 0 {
        out.push(format!("({more} more rows)"));
    }
    out.join("\n")
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
                timeline: None,
                steps: None,
                hunks: None,
                form: None,
                custom: None,
                group: None,
                emphasis: None,
                depth: None,
                children: Vec::new(),
                launcher_category: None,
                hidden: false,
                actions: Vec::new(),
                alert: None,
                path: String::new(),
            },
            params: BTreeMap::new(),
            result: SqlQueryResult {
                columns: columns.iter().map(|c| c.to_string()).collect(),
                rows,
                truncated: false,
                reads: Reads::default(),
                freshness: Vec::new(),
            },
            alert: None,
            warnings: Vec::new(),
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

    /// A grid reads as its children, in order, under their titles.
    #[test]
    fn a_grid_renders_its_children_in_order() {
        let grid = run(LensViz::Grid, None, &[], Vec::new());
        let mut a = run(LensViz::Number, None, &["n"], vec![vec![SqlCell::Int(5)]]);
        a.lens.title = "Open".into();
        let mut b = run(LensViz::Markdown, None, &["m"], vec![vec![t("**ok**")]]);
        b.lens.title = "Note".into();
        assert_eq!(
            render(
                &grid,
                &Resolved {
                    children: vec![a, b],
                    diffs: Vec::new()
                }
            ),
            "### Open\n\n5\n\n### Note\n\n**ok**"
        );
    }

    fn with<F: FnOnce(&mut Lens)>(mut r: LensRun, f: F) -> LensRun {
        f(&mut r.lens);
        r
    }

    /// tsk1089: a grouped list reads as a section per group, in the order
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
                });
                l.depth = Some("d".into());
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "### Ready\n\n| title |\n| --- |\n| a |\n| c |\n\n### Done\n\n| title |\n| --- |\n| b |\n"
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
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- root\n  - child\n    - grandchild\n- orphan"
        );
    }

    /// A timeline lists its rows oldest first, with their refs.
    #[test]
    fn a_timeline_is_in_time_order() {
        let r = with(
            run(
                LensViz::Timeline,
                None,
                &["at", "what", "ref"],
                vec![
                    vec![t("2026-09-30T10:00"), t("shipped"), t("commit:abc")],
                    vec![t("2026-09-29T09:00"), t("started"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.timeline = Some(crate::extensions::LensTimeline {
                    at: Some("at".into()),
                    label: Some("what".into()),
                    ref_column: Some("ref".into()),
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- 2026-09-29T09:00 — started\n- 2026-09-30T10:00 — shipped (commit:abc)"
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
            children: Vec::new(),
            diffs: vec!["-old\n+new\n".into()],
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
}
