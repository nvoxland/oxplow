//! Every lens has a text rendering (design rule 17,
//! `.context/target-architecture.md` §11.3): what an agent reads instead
//! of the rows, what `copy` puts on the clipboard, and what a panel says
//! when read as text. One renderer per kit component, each saying what
//! the component shows — a chart's series, a treemap's sizes, a grid's
//! children in order — capped at [`MAX_ROWS`] rows.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{Reads, SqlCell};
use oxplow_domain::DomainError;
use serde::Serialize;

use crate::extensions::{AlertState, LensRun, LensViz};

/// Rows a table-like rendering shows before saying how many more there
/// are.
pub const MAX_ROWS: usize = 50;

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

/// `run` as an agent reads it; a grid's children are run (with the
/// params each declares from the grid's) so their text is in it.
pub async fn text_run(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<LensText, DomainError> {
    let text = text_of(layer, catalog, root, run, ctx).await?;
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

/// `run`'s text, running a grid's children first.
pub async fn text_of(
    layer: &crate::sql_gateway::SqlGateway,
    catalog: &crate::extension_catalog::ExtensionCatalog,
    root: &Path,
    run: &LensRun,
    ctx: &crate::extensions::LensContext,
) -> Result<String, DomainError> {
    if run.lens.viz != LensViz::Grid {
        return Ok(render(run, &[]));
    }
    let mut children = Vec::with_capacity(run.lens.children.len());
    for id in &run.lens.children {
        let child = catalog.find_lens(root, id)?;
        let params = run
            .params
            .iter()
            .filter(|(k, _)| child.params.iter().any(|p| &p.name == *k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        children.push(crate::extensions::run_lens(layer, catalog, root, id, params, ctx).await?);
    }
    Ok(render(run, &children))
}

/// `run` as text. A grid renders `children` (its children's runs, in
/// order) under their titles; every other component renders its rows.
pub fn render(run: &LensRun, children: &[LensRun]) -> String {
    let rows = &run.result.rows;
    if run.lens.viz == LensViz::Grid {
        return children
            .iter()
            .map(|c| format!("### {}\n\n{}", c.lens.title, render(c, &[])))
            .collect::<Vec<_>>()
            .join("\n\n");
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
        LensViz::Table | LensViz::List | LensViz::Grid => table(run),
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
/// their labels), or every result column when none are declared.
fn table(run: &LensRun) -> String {
    let shown: Vec<(usize, String)> = if run.lens.columns.is_empty() {
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
    let header: Vec<String> = shown.iter().map(|(_, l)| l.clone()).collect();
    let rows: Vec<Vec<String>> = run
        .result
        .rows
        .iter()
        .map(|r| {
            shown
                .iter()
                .map(|(i, _)| r.get(*i).map(cell).unwrap_or_default())
                .collect()
        })
        .collect();
    markdown_table(&header, &rows)
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
            render(&r, &[]),
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
            render(&r, &[]),
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
            render(&r, &[]),
            "| path | lines |\n| --- | --- |\n| b.rs | 30 |\n| a.rs | 10 |\n"
        );
    }

    /// A table stops at MAX_ROWS and says how many more; an empty result
    /// says the lens's `empty` text.
    #[test]
    fn a_table_is_capped_and_an_empty_one_says_so() {
        let rows = (0..60).map(|i| vec![SqlCell::Int(i)]).collect();
        let text = render(&run(LensViz::Table, None, &["n"], rows), &[]);
        assert!(text.ends_with("(10 more rows)\n"), "{text}");
        assert!(text.contains("| 49 |") && !text.contains("| 50 |"));
        let mut empty = run(LensViz::Table, None, &["n"], Vec::new());
        assert_eq!(render(&empty, &[]), "No rows.");
        empty.lens.empty = Some("Nothing waiting.".into());
        assert_eq!(render(&empty, &[]), "Nothing waiting.");
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
            render(&grid, &[a, b]),
            "### Open\n\n5\n\n### Note\n\n**ok**"
        );
    }
}
