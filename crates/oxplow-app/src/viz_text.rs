//! Every viz has a text rendering (design rule 17,
//! `.context/extensions-pages.md`): what an agent reads instead
//! of the rows, what `copy` puts on the clipboard, and what a panel says
//! when read as text. One renderer per kit component, each saying what
//! the component shows — a chart's series, a treemap's sizes, a grid's
//! children in order, a tree's nesting, a hunk's diff — capped at
//! [`MAX_ROWS`] rows.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::SqlCell;
use oxplow_domain::DomainError;

use crate::pages::spec::{ColumnFormat, Viz, VizRun};

/// Rows a table-like rendering shows before saying how many more there
/// are.
pub const MAX_ROWS: usize = 50;

/// Files a `hunks` rendering diffs, and the bytes of each file's diff.
pub const MAX_HUNK_FILES: usize = 20;
pub const MAX_HUNK_BYTES: usize = 8_000;

/// What a component needs beyond its rows: a `hunks` viz's diffs (one per
/// row, in row order, up to [`MAX_HUNK_FILES`]) or a `diff` viz's one, a
/// `code` viz's files, an `approvals` viz's proposals.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub diffs: Vec<String>,
    /// A `code` viz's files, one per row (up to [`MAX_HUNK_FILES`]): each
    /// one's text at its revision, cut to its marked lines.
    pub files: Vec<String>,
    /// An `approvals` viz's pending proposals, one line each.
    pub approvals: Vec<String>,
}

/// `run`'s text, resolved first.
pub async fn text_of(
    svc: &crate::Services,
    root: &Path,
    run: &VizRun,
) -> Result<String, DomainError> {
    Ok(render(run, &resolve(svc, root, run).await?))
}

/// What `run`'s component needs beyond its rows: a `hunks` or `diff`
/// viz's diffs, each file read at both revisions in `root`, a `code` or
/// `editor` viz's files, an `approvals` viz's proposals.
pub async fn resolve(
    svc: &crate::Services,
    root: &Path,
    run: &VizRun,
) -> Result<Resolved, DomainError> {
    let mut out = Resolved::default();
    match run.config.viz {
        Viz::Approvals => out.approvals = pending_proposals(svc, run).await?,
        Viz::Code => {
            let c = run.config.code.clone().unwrap_or_default();
            let Some(p) = column(run, c.path.as_ref()) else {
                return Ok(out);
            };
            let (r, f) = (
                column(run, c.revision.as_ref()),
                column(run, c.filesystem.as_ref()),
            );
            let l = column(run, c.lines.as_ref());
            for row in run.result.rows.iter().take(MAX_HUNK_FILES) {
                let at = |i: usize| row.get(i).map(cell).unwrap_or_default();
                let lines = l.and_then(|i| parse_lines(&at(i)));
                let rev = r.map(at).unwrap_or_default();
                let filesystem = f.map(at).unwrap_or_default();
                out.files
                    .push(file_text(svc, root, &filesystem, &at(p), &rev, lines).await);
            }
        }
        Viz::Editor => {
            let e = run.config.editor.clone().unwrap_or_default();
            let (Some(p), Some(row)) = (column(run, e.path.as_ref()), run.result.rows.first())
            else {
                return Ok(out);
            };
            let path = row.get(p).map(cell).unwrap_or_default();
            out.files
                .push(file_text(svc, root, "", &path, "working", None).await);
        }
        Viz::Diff => {
            let d = run.config.diff.clone().unwrap_or_default();
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
        Viz::Hunks => {
            let h = run.config.hunks.clone().unwrap_or_default();
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

/// The pending proposals an `approvals` viz shows, one line each: its
/// params `thread` (a thread ref) and `setting` (a config key) narrow them.
async fn pending_proposals(
    svc: &crate::Services,
    run: &VizRun,
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

/// `path` at revision `rev` of `filesystem` (empty: the local one, whose
/// empty revision is the working tree), cut to `lines` when given and to
/// [`MAX_HUNK_BYTES`], or why it can't be shown.
async fn file_text(
    svc: &crate::Services,
    root: &Path,
    filesystem: &str,
    path: &str,
    rev: &str,
    lines: Option<(usize, usize)>,
) -> String {
    let bytes = if filesystem.is_empty() || filesystem == oxplow_domain::filesystem::LOCAL {
        let parsed: oxplow_domain::vcs::Revision =
            match if rev.is_empty() { "working" } else { rev }.parse() {
                Ok(r) => r,
                Err(e) => return format!("(can't read it: {e})"),
            };
        svc.trees
            .read_at(root, &parsed, path)
            .await
            .map_err(|e| e.to_string())
    } else {
        let cx = oxplow_domain::filesystem::FsCx { stream: None };
        match svc.filesystems.resolve(Some(filesystem)) {
            Ok(fs) => fs
                .read(&cx, (!rev.is_empty()).then_some(rev), path, None)
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    };
    let text = match bytes {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => {
            return format!(
                "(not there at {})",
                if rev.is_empty() { filesystem } else { rev }
            )
        }
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

/// `run` as text: a `hunks` viz its resolved diffs; every other component
/// renders its rows.
pub fn render(run: &VizRun, resolved: &Resolved) -> String {
    let rows = &run.result.rows;
    if run.config.viz == Viz::Form {
        // An agent runs the command itself; the text says which.
        let f = run.config.form.clone().unwrap_or_default();
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
            .config
            .empty
            .clone()
            .unwrap_or_else(|| "No rows.".to_string());
    }
    let body = match run.config.viz {
        Viz::Markdown => {
            // Its `text` column (else the first) of the first row.
            let at = run
                .config
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
        Viz::Number => rows
            .first()
            .and_then(|r| r.first())
            .map(cell)
            .unwrap_or_default(),
        Viz::Bar => bar(run),
        Viz::Line => line(run),
        Viz::Treemap => treemap(run),
        Viz::Tree => tree(run),
        Viz::Timeline => timeline(run),
        Viz::Detail => detail(run),
        Viz::Sections => sections(run),
        // A board reads as its columns, each a section of its cards.
        Viz::Board => table(run),
        Viz::Steps => steps(run),
        Viz::Hunks => hunks(run, &resolved.diffs),
        Viz::Code => code(run, &resolved.files),
        Viz::Editor => editor(run, &resolved.files),
        Viz::Terminal => terminals(run),
        Viz::Embed => embed(run),
        Viz::Media => media(run),
        Viz::Diff => one_diff(run, &resolved.diffs),
        Viz::Approvals => {
            if resolved.approvals.is_empty() {
                "No proposals wait for a decision.".to_string()
            } else {
                format!(
                    "Proposals waiting for a person's decision (an agent can't decide them):\n{}",
                    resolved.approvals.join("\n")
                )
            }
        }
        Viz::Table | Viz::List | Viz::Form => table(run),
        // A component's rendering is its own; an agent reads its rows.
        Viz::Custom => format!(
            "(custom component `{}/{}`; its table rendering)\n{}",
            run.config.extension,
            run.config
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

/// An `items` cell: a JSON array of strings and `{ text, hint? }` objects
/// as `text (hint) · text`. A value that isn't such a list reads as it is.
fn items(c: &SqlCell) -> String {
    let raw = cell(c);
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return raw;
    };
    list.iter()
        .filter_map(|item| match item {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Object(o) => {
                let text = o.get("text")?.as_str()?;
                Some(match o.get("hint").and_then(|h| h.as_str()) {
                    Some(hint) if !hint.is_empty() => format!("{text} ({hint})"),
                    _ => text.to_string(),
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Row `r`'s value in result column `i`, as the person reads it: an
/// `items` column as its items.
fn value(run: &VizRun, r: &[SqlCell], i: usize) -> String {
    let Some(c) = r.get(i) else {
        return String::new();
    };
    let is_items = run.result.columns.get(i).is_some_and(|key| {
        run.config
            .columns
            .iter()
            .any(|c| &c.key == key && c.format == Some(ColumnFormat::Items))
    });
    if is_items {
        items(c)
    } else {
        cell(c)
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
fn table(run: &VizRun) -> String {
    let shown = shown(run);
    let header: Vec<String> = shown.iter().map(|(_, l)| l.clone()).collect();
    let line = |r: &Vec<SqlCell>| -> Vec<String> {
        shown.iter().map(|(i, _)| value(run, r, *i)).collect()
    };
    let by = run
        .config
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
        .config
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
fn shown(run: &VizRun) -> Vec<(usize, String)> {
    let styling: Vec<&String> = run
        .config
        .group
        .iter()
        .map(|g| &g.by)
        .chain(run.config.emphasis.iter())
        .chain(run.config.depth.iter())
        .collect();
    let all: Vec<(usize, String)> = if run.config.columns.is_empty() {
        run.result.columns.iter().cloned().enumerate().collect()
    } else {
        run.config
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
fn tree(run: &VizRun) -> String {
    let t = run.config.tree.clone().unwrap_or_default();
    let (Some(id), Some(parent), Some(label)) = (
        column(run, t.id.as_ref()),
        column(run, t.parent.as_ref()),
        column(run, t.label.as_ref()),
    ) else {
        return table(run);
    };
    // A lazy tree's run is one level: a row the `branch` column marks
    // opens by running the viz again with `lazy` set to its id.
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
            "\n\nA ▸ row opens with `{param}=<id>`: run the viz with it to read that level (this run: `{param}={now}`)."
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
fn timeline(run: &VizRun) -> String {
    let t = run.config.timeline.clone().unwrap_or_default();
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

/// A detail: the first row's displayed columns as label/value lines. A
/// column with a `when` is a fact only while that column of the row is
/// truthy.
fn detail(run: &VizRun) -> String {
    let Some(row) = run.result.rows.first() else {
        return String::new();
    };
    let drawn = |i: usize| {
        let key = &run.result.columns[i];
        let when = run
            .config
            .columns
            .iter()
            .find(|c| &c.key == key)
            .and_then(|c| c.when.as_ref());
        match when.and_then(|w| run.result.columns.iter().position(|k| k == w)) {
            Some(at) => row.get(at).is_some_and(truthy),
            None => true,
        }
    };
    shown(run)
        .into_iter()
        .filter(|(i, _)| drawn(*i))
        .map(|(i, l)| format!("**{l}**: {}", value(run, row, i)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sections: per row, a `###` heading (its link after it), the row's
/// displayed `columns` as label/value lines, then its markdown body — as
/// the person reads the subsections. The role columns are not facts. With
/// a `group`, a `###` heading per group (its link after it, groups in the
/// order each first appears) and the rows' headings one level deeper; a
/// row with no group stays at the groups' level.
fn sections(run: &VizRun) -> String {
    let s = run.config.sections.clone().unwrap_or_default();
    let Some(title) = column(run, s.title.as_ref()) else {
        return table(run);
    };
    let link = column(run, s.link.as_ref());
    let body = column(run, s.body.as_ref());
    let group = column(run, s.group.as_ref());
    let group_link = column(run, s.group_link.as_ref());
    let role: Vec<usize> = [
        Some(title),
        link,
        body,
        column(run, s.id.as_ref()),
        group,
        group_link,
    ]
    .into_iter()
    .flatten()
    .collect();
    let facts: Vec<(usize, String)> = shown(run)
        .into_iter()
        .filter(|(i, _)| !role.contains(i))
        .collect();
    let block = |r: &Vec<SqlCell>, hashes: &str| {
        let at = |i: usize| Some(value(run, r, i)).filter(|v| !v.is_empty());
        let mut out = format!("{hashes} {}", at(title).unwrap_or_default());
        if let Some(uri) = link.and_then(at) {
            out.push_str(&format!(" ({uri})"));
        }
        for (i, label) in &facts {
            out.push_str(&format!("\n**{label}**: {}", at(*i).unwrap_or_default()));
        }
        if let Some(text) = body.and_then(at) {
            out.push_str(&format!("\n\n{text}"));
        }
        out
    };
    let named = |r: &Vec<SqlCell>| group.map(|g| value(run, r, g)).filter(|v| !v.is_empty());
    // Each group once, in the order it first appears, with its rows; a row
    // with no group is a block of its own where it falls.
    enum Part<'a> {
        Group(String, &'a Vec<SqlCell>, Vec<&'a Vec<SqlCell>>),
        Loose(&'a Vec<SqlCell>),
    }
    let mut parts: Vec<Part> = Vec::new();
    for r in &run.result.rows {
        match named(r) {
            Some(name) => match parts.iter_mut().find_map(|p| match p {
                Part::Group(n, _, rows) if *n == name => Some(rows),
                _ => None,
            }) {
                Some(rows) => rows.push(r),
                None => parts.push(Part::Group(name, r, vec![r])),
            },
            None => parts.push(Part::Loose(r)),
        }
    }
    let blocks: Vec<String> = parts
        .iter()
        .map(|p| match p {
            Part::Loose(r) => block(r, "###"),
            Part::Group(name, first, rows) => {
                let mut out = format!("### {name}");
                if let Some(uri) = group_link
                    .map(|g| value(run, first, g))
                    .filter(|v| !v.is_empty())
                {
                    out.push_str(&format!(" ({uri})"));
                }
                for r in rows {
                    out.push_str("\n\n");
                    out.push_str(&block(r, "####"));
                }
                out
            }
        })
        .collect();
    let more = blocks.len().saturating_sub(MAX_ROWS);
    let mut out: Vec<String> = blocks.into_iter().take(MAX_ROWS).collect();
    if more > 0 {
        out.push(format!("({more} more rows)"));
    }
    out.join("\n\n")
}

/// Steps: a numbered checklist — `[x]` done, `[>]` active, `[!]` failed,
/// `[ ]` anything else.
fn steps(run: &VizRun) -> String {
    let s = run.config.steps.clone().unwrap_or_default();
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
fn hunks(run: &VizRun, diffs: &[String]) -> String {
    let h = run.config.hunks.clone().unwrap_or_default();
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
/// The terminals a viz shows, one line each. Their output isn't read here
/// (an agent never reads a person's terminal; a session's transcript is).
fn terminals(run: &VizRun) -> String {
    let t = run.config.terminal.clone().unwrap_or_default();
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
fn editor(run: &VizRun, files: &[String]) -> String {
    let e = run.config.editor.clone().unwrap_or_default();
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
fn embed(run: &VizRun) -> String {
    let e = run.config.embed.clone().unwrap_or_default();
    let (Some(u), Some(row)) = (column(run, e.url.as_ref()), run.result.rows.first()) else {
        return table(run);
    };
    let url = row.get(u).map(cell).unwrap_or_default();
    format!("[{url}]({url}) — a web page, opened in its own window")
}

/// Media is a line per file saying where it is and what it is: text can't
/// show an image or a PDF, so the agent learns the file to read instead.
fn media(run: &VizRun) -> String {
    let m = run.config.media.clone().unwrap_or_default();
    let Some(p) = column(run, m.path.as_ref()) else {
        return table(run);
    };
    let (f, r) = (
        column(run, m.filesystem.as_ref()),
        column(run, m.revision.as_ref()),
    );
    run.result
        .rows
        .iter()
        .map(|row| {
            let at = |i: Option<usize>| i.and_then(|i| row.get(i)).map(cell).unwrap_or_default();
            let path = at(Some(p));
            let what = if is_pdf(&path) { "a PDF" } else { "an image" };
            let fs = at(f);
            let rev = at(r);
            let where_ = match (fs.as_str(), rev.as_str()) {
                ("" | "oxplow:local", "") => String::new(),
                ("" | "oxplow:local", rev) => format!("@{rev}"),
                (fs, "") => format!(" on {fs}"),
                (fs, rev) => format!(" on {fs}@{rev}"),
            };
            format!("- {path}{where_} — {what}, shown in the app")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_pdf(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
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

fn code(run: &VizRun, files: &[String]) -> String {
    let c = run.config.code.clone().unwrap_or_default();
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
fn one_diff(run: &VizRun, diffs: &[String]) -> String {
    let d = run.config.diff.clone().unwrap_or_default();
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

fn column(run: &VizRun, name: Option<&String>) -> Option<usize> {
    let name = name?;
    run.result.columns.iter().position(|c| c == name)
}

/// Bars: each label and its value, and the total.
fn bar(run: &VizRun) -> String {
    let chart = run.config.chart.clone().unwrap_or_default();
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
fn line(run: &VizRun) -> String {
    let chart = run.config.chart.clone().unwrap_or_default();
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
fn treemap(run: &VizRun) -> String {
    let chart = run.config.chart.clone().unwrap_or_default();
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
    use crate::pages::spec::{VizChart, VizConfig};
    use oxplow_db::SqlQueryResult;

    /// A markdown viz reads as its `text` column, not its first.
    #[test]
    fn a_markdown_viz_reads_as_its_text_column() {
        let mut r = run(
            Viz::Markdown,
            None,
            &["slug", "body"],
            vec![vec![t("a"), t("# Hello")]],
        );
        r.config.markdown = Some(crate::pages::spec::VizMarkdown {
            text: Some("body".into()),
            base: None,
        });
        let out = render(&r, &Resolved::default());
        assert!(out.contains("# Hello") && !out.contains("\na\n"), "{out}");
    }

    fn run(viz: Viz, chart: Option<VizChart>, columns: &[&str], rows: Vec<Vec<SqlCell>>) -> VizRun {
        VizRun {
            config: VizConfig {
                id: "t/l".into(),
                extension: "t".into(),
                slug: "l".into(),
                title: "VizConfig".into(),
                viz,
                params: Vec::new(),
                columns: Vec::new(),
                empty: None,
                fold: false,
                chart,
                tree: None,
                timeline: None,
                steps: None,
                sections: None,
                hunks: None,
                code: None,
                media: None,
                diff: None,
                editor: None,
                embed: None,
                terminal: None,
                markdown: None,
                form: None,
                custom: None,
                group: None,
                emphasis: None,
                depth: None,
                fills: false,
                actions: Vec::new(),
                origin: Default::default(),
                lead: None,
            },
            params: BTreeMap::new(),
            result: SqlQueryResult {
                columns: columns.iter().map(|c| c.to_string()).collect(),
                rows,
                truncated: false,
                reads: oxplow_db::Reads::default(),
                freshness: Vec::new(),
                generation: None,
                coverage: Vec::new(),
            },
        }
    }

    fn t(s: &str) -> SqlCell {
        SqlCell::Text(s.into())
    }

    fn chart(x: &str, y: &str, series: Option<&str>) -> Option<VizChart> {
        Some(VizChart {
            x: Some(x.into()),
            y: Some(y.into()),
            series: series.map(str::to_string),
            ..VizChart::default()
        })
    }

    /// A bar chart reads as its labels and values, with the total.
    #[test]
    fn a_bar_viz_is_a_series_table_with_its_total() {
        let r = run(
            Viz::Bar,
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
    fn a_line_viz_has_a_column_per_series() {
        let r = run(
            Viz::Line,
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
            Viz::Treemap,
            Some(VizChart {
                label: Some("path".into()),
                size: Some("lines".into()),
                ..VizChart::default()
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
    /// says the viz's `empty` text.
    #[test]
    fn a_table_is_capped_and_an_empty_one_says_so() {
        let rows = (0..60).map(|i| vec![SqlCell::Int(i)]).collect();
        let text = render(&run(Viz::Table, None, &["n"], rows), &Resolved::default());
        assert!(text.ends_with("(10 more rows)\n"), "{text}");
        assert!(text.contains("| 49 |") && !text.contains("| 50 |"));
        let mut empty = run(Viz::Table, None, &["n"], Vec::new());
        assert_eq!(render(&empty, &Resolved::default()), "No rows.");
        empty.config.empty = Some("Nothing waiting.".into());
        assert_eq!(render(&empty, &Resolved::default()), "Nothing waiting.");
    }

    fn with<F: FnOnce(&mut VizConfig)>(mut r: VizRun, f: F) -> VizRun {
        f(&mut r.config);
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
        let r = with(run(Viz::List, None, &["bucket", "title", "d"], rows), |l| {
            l.group = Some(crate::pages::spec::VizGroup {
                by: "bucket".into(),
                link: None,
                values: Vec::new(),
                labels: Default::default(),
            });
            l.depth = Some("d".into());
        });
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
        let r = with(run(Viz::Board, None, &["state", "title"], rows), |l| {
            l.group = Some(crate::pages::spec::VizGroup {
                by: "state".into(),
                link: None,
                values: vec!["todo".into(), "done".into()],
                labels: Default::default(),
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
                Viz::Tree,
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
                l.tree = Some(crate::pages::spec::VizTree {
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
                Viz::Tree,
                None,
                &["id", "parent", "name", "is_dir"],
                vec![
                    vec![t("src"), t(""), t("src"), SqlCell::Int(1)],
                    vec![t("a.rs"), t(""), t("a.rs"), SqlCell::Int(0)],
                ],
            ),
            |l| {
                l.tree = Some(crate::pages::spec::VizTree {
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
            "- ▸ src\n- a.rs\n\nA ▸ row opens with `folder=<id>`: run the viz with it to read that level (this run: `folder=`)."
        );
    }

    /// A timeline lists its rows oldest first, with their refs.
    #[test]
    fn a_timeline_is_in_time_order() {
        let r = with(
            run(
                Viz::Timeline,
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
                l.timeline = Some(crate::pages::spec::VizTimeline {
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
            Viz::Detail,
            None,
            &["title", "state"],
            vec![vec![t("Fix it"), t("done")], vec![t("ignored"), t("x")]],
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "**title**: Fix it\n**state**: done"
        );
    }

    /// A fact whose column has a `when` is left out while that column of
    /// the row is falsy: a merge's second parent on a commit that isn't one.
    #[test]
    fn a_detail_leaves_out_a_fact_whose_when_column_is_falsy() {
        let col = |key: &str, when: Option<&str>| crate::pages::spec::PageColumn {
            key: key.into(),
            label: None,
            link: None,
            unit: None,
            icon: None,
            tone: None,
            width: None,
            format: None,
            highlight: None,
            select: None,
            expand: None,
            when: when.map(str::to_string),
        };
        let one = with(
            run(
                Viz::Detail,
                None,
                &["parent", "merged"],
                vec![vec![t("abc"), SqlCell::Null(())]],
            ),
            |l| l.columns = vec![col("parent", None), col("merged", Some("merged"))],
        );
        assert_eq!(render(&one, &Resolved::default()), "**parent**: abc");
        let merge = with(
            run(
                Viz::Detail,
                None,
                &["parent", "merged"],
                vec![vec![t("abc"), t("def")]],
            ),
            |l| l.columns = vec![col("parent", None), col("merged", Some("merged"))],
        );
        assert_eq!(
            render(&merge, &Resolved::default()),
            "**parent**: abc\n**merged**: def"
        );
    }

    /// Sections read as the person sees them: a heading per row (its link
    /// after it), its facts as label/value lines, then its markdown body;
    /// the role columns are not facts.
    #[test]
    fn sections_are_a_heading_per_row_with_facts_and_body() {
        let r = with(
            run(
                Viz::Sections,
                None,
                &["id", "name", "uri", "kind", "note"],
                vec![
                    vec![
                        t("a"),
                        t("Alpha"),
                        t("oxplow:x/a"),
                        t("ships"),
                        t("Says **hi**."),
                    ],
                    vec![
                        t("b"),
                        t("Beta"),
                        SqlCell::Null(()),
                        t("local"),
                        SqlCell::Null(()),
                    ],
                ],
            ),
            |l| {
                l.sections = Some(crate::pages::spec::VizSections {
                    title: Some("name".into()),
                    id: Some("id".into()),
                    link: Some("uri".into()),
                    body: Some("note".into()),
                    ..Default::default()
                });
                l.columns = vec![crate::pages::spec::PageColumn {
                    key: "kind".into(),
                    label: Some("Kind".into()),
                    link: None,
                    unit: None,
                    icon: None,
                    tone: None,
                    width: None,
                    format: None,
                    highlight: None,
                    select: None,
                    expand: None,
                    when: None,
                }];
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "### Alpha (oxplow:x/a)\n**Kind**: ships\n\nSays **hi**.\n\n### Beta\n**Kind**: local"
        );
    }

    /// A grouped `sections` viz reads a heading per group (its link after
    /// it), the rows' headings one level deeper, groups in the order each
    /// first appears; a row with no group is a heading at the groups' level.
    #[test]
    fn grouped_sections_nest_their_rows_under_the_group_heading() {
        let r = with(
            run(
                Viz::Sections,
                None,
                &["name", "ext", "ext_uri", "kind"],
                vec![
                    vec![t("P1"), t("Alpha"), t("oxplow:x/alpha"), t("a")],
                    vec![t("Q1"), t("Beta"), SqlCell::Null(()), t("b")],
                    vec![t("P2"), t("Alpha"), t("oxplow:x/alpha"), t("c")],
                    vec![t("Loose"), SqlCell::Null(()), SqlCell::Null(()), t("d")],
                ],
            ),
            |l| {
                l.sections = Some(crate::pages::spec::VizSections {
                    title: Some("name".into()),
                    group: Some("ext".into()),
                    group_link: Some("ext_uri".into()),
                    ..Default::default()
                });
                l.columns = vec![crate::pages::spec::PageColumn {
                    key: "kind".into(),
                    label: Some("Kind".into()),
                    link: None,
                    unit: None,
                    icon: None,
                    tone: None,
                    width: None,
                    format: None,
                    highlight: None,
                    select: None,
                    expand: None,
                    when: None,
                }];
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "### Alpha (oxplow:x/alpha)\n\n#### P1\n**Kind**: a\n\n#### P2\n**Kind**: c\n\n### Beta\n\n#### Q1\n**Kind**: b\n\n### Loose\n**Kind**: d"
        );
    }

    /// An `items` column reads as its items joined by " · ", a hint in
    /// parentheses; a value that is not a list of items reads as it is.
    #[test]
    fn an_items_column_reads_as_its_items_with_hints() {
        let items = r#"["plain",{"text":"sql.read","hint":"may run its own SQL"},{"text":"to","uri":"oxplow:x/y"}]"#;
        let r = with(
            run(
                Viz::Detail,
                None,
                &["perms", "other"],
                vec![vec![t(items), t("[1]")]],
            ),
            |l| {
                let col = |key: &str, format| crate::pages::spec::PageColumn {
                    key: key.into(),
                    label: None,
                    link: None,
                    unit: None,
                    icon: None,
                    tone: None,
                    width: None,
                    format,
                    highlight: None,
                    select: None,
                    expand: None,
                    when: None,
                };
                l.columns = vec![
                    col("perms", Some(crate::pages::spec::ColumnFormat::Items)),
                    col("other", None),
                ];
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "**perms**: plain · sql.read (may run its own SQL) · to\n**other**: [1]"
        );
    }

    /// Steps are a numbered checklist marked by status.
    #[test]
    fn steps_are_a_checklist() {
        let r = with(
            run(
                Viz::Steps,
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
                l.steps = Some(crate::pages::spec::VizSteps {
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
                Viz::Hunks,
                None,
                &["path", "a", "b"],
                vec![
                    vec![t("x.rs"), t("git:abc"), t("working")],
                    vec![t("y.rs"), t("git:abc"), t("working")],
                ],
            ),
            |l| {
                l.hunks = Some(crate::pages::spec::VizHunks {
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

    /// A hunks viz resolves each row's diff from the workspace.
    #[tokio::test]
    async fn hunks_resolve_from_the_workspace() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("h.txt"), "one\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "h");
        std::fs::write(root.join("h.txt"), "two\n").unwrap();
        let r = with(
            run(
                Viz::Hunks,
                None,
                &["path", "a", "b"],
                vec![vec![t("h.txt"), t(&format!("git:{sha}")), t("working")]],
            ),
            |l| {
                l.hunks = Some(crate::pages::spec::VizHunks {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                })
            },
        );
        let text = text_of(&fx.svc, &root, &r).await.unwrap();
        assert!(text.contains("-one") && text.contains("+two"), "{text}");
    }

    /// Code is each row's file, revision and marked lines, then its text
    /// in a fence named for its extension; files past the resolved ones
    /// are counted.
    #[test]
    fn code_shows_each_files_lines() {
        let r = with(
            run(
                Viz::Code,
                None,
                &["path", "rev", "lines"],
                vec![
                    vec![t("src/x.rs"), t("git:abc"), t("8-3")],
                    vec![t("y.rs"), t("working"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.code = Some(crate::pages::spec::VizCode {
                    path: Some("path".into()),
                    filesystem: None,
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

    /// Media is each row's file and revision, said as what it is: text
    /// can't show an image or a PDF, so the line says where it is.
    #[test]
    fn media_says_what_each_file_is() {
        let r = with(
            run(
                Viz::Media,
                None,
                &["path", "fs", "rev"],
                vec![
                    vec![t("img/logo.png"), SqlCell::Null(()), t("working")],
                    vec![t("docs/spec.pdf"), t("oxplow:temp"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.media = Some(crate::pages::spec::VizMedia {
                    path: Some("path".into()),
                    filesystem: Some("fs".into()),
                    revision: Some("rev".into()),
                })
            },
        );
        assert_eq!(
            render(&r, &Resolved::default()),
            "- img/logo.png@working — an image, shown in the app\n- docs/spec.pdf on oxplow:temp — a PDF, shown in the app"
        );
    }

    /// A diff is the first row's file and revisions, then its diff.
    #[test]
    fn a_diff_shows_the_first_rows_diff() {
        let r = with(
            run(
                Viz::Diff,
                None,
                &["path", "a", "b"],
                vec![
                    vec![t("x.rs"), t("git:abc"), t("working")],
                    vec![t("y.rs"), t("git:abc"), t("working")],
                ],
            ),
            |l| {
                l.diff = Some(crate::pages::spec::VizDiff {
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

    /// A code viz reads each row's file at its revision, cut to its
    /// lines; a diff viz diffs its first row's file.
    #[tokio::test]
    async fn code_and_diff_resolve_from_the_workspace() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("c.txt"), "one\ntwo\nthree\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "c");
        std::fs::write(root.join("c.txt"), "uno\n").unwrap();
        let code = with(
            run(
                Viz::Code,
                None,
                &["path", "rev", "lines"],
                vec![
                    vec![t("c.txt"), t(&format!("git:{sha}")), t("2-3")],
                    vec![t("gone.txt"), t("working"), SqlCell::Null(())],
                ],
            ),
            |l| {
                l.code = Some(crate::pages::spec::VizCode {
                    path: Some("path".into()),
                    filesystem: None,
                    revision: Some("rev".into()),
                    lines: Some("lines".into()),
                    sync: false,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &code).await.unwrap();
        assert!(text.contains("```txt\ntwo\nthree\n```"), "{text}");
        assert!(text.contains("(not there at working)"), "{text}");
        let diff = with(
            run(
                Viz::Diff,
                None,
                &["path", "a", "b"],
                vec![vec![t("c.txt"), t(&format!("git:{sha}")), t("working")]],
            ),
            |l| {
                l.diff = Some(crate::pages::spec::VizDiff {
                    path: Some("path".into()),
                    from: Some("a".into()),
                    to: Some("b".into()),
                    lines: None,
                    base: None,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &diff).await.unwrap();
        assert!(text.contains("-one") && text.contains("+uno"), "{text}");
    }

    /// An editor viz reads its first row's file as the working tree has
    /// it, and says the person's unsaved edits aren't in it.
    #[tokio::test]
    async fn an_editor_reads_the_working_file() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("e.txt"), "saved text\n").unwrap();
        let editor = with(
            run(Viz::Editor, None, &["path"], vec![vec![t("e.txt")]]),
            |l| {
                l.editor = Some(crate::pages::spec::VizEditor {
                    path: Some("path".into()),
                    filesystem: None,
                    lines: None,
                })
            },
        );
        let text = text_of(&fx.svc, &root, &editor).await.unwrap();
        assert!(
            text.contains("#### e.txt, being edited (as saved; unsaved edits aren't here)\n\n```txt\nsaved text\n```"),
            "{text}"
        );
    }

    /// A terminal viz names its terminals and never their output.
    #[test]
    fn terminals_read_as_their_names() {
        let r = with(
            run(
                Viz::Terminal,
                None,
                &["uri", "title"],
                vec![
                    vec![t("oxplow:oxplow/terminal?id=trm1"), t("Build")],
                    vec![t("oxplow:oxplow/agent_session?id=ses2"), t("Claude")],
                ],
            ),
            |l| {
                l.terminal = Some(crate::pages::spec::VizTerminal {
                    of: Some("uri".into()),
                    title: Some("title".into()),
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

    /// An embed viz reads as a link to its page.
    #[tokio::test]
    async fn an_embed_reads_as_a_link() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let embed = with(
            run(
                Viz::Embed,
                None,
                &["url"],
                vec![vec![t("https://example.com/a")]],
            ),
            |l| {
                l.embed = Some(crate::pages::spec::VizEmbed {
                    url: Some("url".into()),
                })
            },
        );
        let text = text_of(&fx.svc, &root, &embed).await.unwrap();
        assert!(
            text.contains("[https://example.com/a](https://example.com/a)"),
            "{text}"
        );
    }

    /// An approvals viz reads the pending proposals itself: an agent
    /// sees what waits (and that it can't decide), narrowed by the
    /// viz's `thread` param; with none waiting it says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn approvals_list_the_pending_proposals() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let approvals = |params: &[(&str, SqlCell)]| {
            let mut r = run(Viz::Approvals, None, &["n"], vec![vec![SqlCell::Int(1)]]);
            for (k, v) in params {
                r.params.insert(k.to_string(), v.clone());
            }
            r
        };
        let none = text_of(&fx.svc, &root, &approvals(&[])).await.unwrap();
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
        let all = text_of(&fx.svc, &root, &approvals(&[])).await.unwrap();
        assert!(
            all.contains("waiting for a person's decision")
                && all.contains("oxplow:oxplow/proposal?id="),
            "{all}"
        );
        let other = text_of(
            &fx.svc,
            &root,
            &approvals(&[("thread", t("oxplow:oxplow/thread?id=thr999"))]),
        )
        .await
        .unwrap();
        assert_eq!(other, "No proposals wait for a decision.");
    }
}
