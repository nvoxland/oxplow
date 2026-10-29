//! What a tab of a page says: the Rust twin of the desktop's `tabLabel`
//! (`apps/desktop/src/pages/renderers.ts`), so a fixture's `expect.label`
//! checks the text the person reads. The desktop host draws the real tab;
//! change both together.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;

use super::runner::PageRun;
use super::schema::Page;

/// `page`'s `label` with each `{{param.<name>}}` filled from `params` and
/// each `{{row.<column>}}` from `row` (its first loader's first row), each
/// through its filters (`short`, `basename`, `url`); else — no label, one
/// that fills to nothing, or one that names the row before there is one —
/// its title.
pub fn tab_label(
    page: &Page,
    params: &BTreeMap<String, SqlCell>,
    row: Option<&BTreeMap<String, SqlCell>>,
) -> String {
    let label = page.label.as_deref().unwrap_or_default();
    let reads_row = regex::Regex::new(r"\{\{\s*row\.")
        .expect("a pattern")
        .is_match(label);
    if reads_row && row.is_none() {
        return page.title.clone();
    }
    let pattern =
        regex::Regex::new(r"\{\{\s*(param|row)\.([a-z0-9_]+)((?:\s*\|\s*[a-z]+)*)\s*\}\}")
            .expect("a pattern");
    let filled = pattern.replace_all(label, |c: &regex::Captures<'_>| {
        let source = if &c[1] == "row" { row } else { Some(params) };
        let mut text = source
            .and_then(|s| s.get(&c[2]))
            .map(show)
            .unwrap_or_default();
        for filter in c[3].split('|').map(str::trim).filter(|f| !f.is_empty()) {
            text = filtered(filter, text);
        }
        text
    });
    let filled = filled.trim();
    if filled.is_empty() {
        page.title.clone()
    } else {
        filled.to_string()
    }
}

/// The label of a run: its params and its first loader's first row.
pub fn run_label(run: &PageRun) -> String {
    let row = run
        .page
        .first_loader()
        .and_then(|l| run.loaders.get(&l.name))
        .and_then(|l| {
            let first = l.result.rows.first()?;
            Some(
                l.result
                    .columns
                    .iter()
                    .cloned()
                    .zip(first.iter().cloned())
                    .collect::<BTreeMap<_, _>>(),
            )
        });
    tab_label(&run.page, &run.params, row.as_ref())
}

fn show(cell: &SqlCell) -> String {
    match cell {
        SqlCell::Null(()) => String::new(),
        SqlCell::Bool(b) => b.to_string(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Text(t) => t.clone(),
    }
}

fn filtered(filter: &str, text: String) -> String {
    match filter {
        "short" => text.chars().take(7).collect(),
        "basename" => text
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&text)
            .to_string(),
        "url" => match url::Url::parse(&text) {
            Ok(u) => {
                let host = u.host_str().unwrap_or_default();
                let host = match u.port() {
                    Some(p) => format!("{host}:{p}"),
                    None => host.to_string(),
                };
                let path = if u.path() == "/" { "" } else { u.path() };
                let shown = format!("{host}{path}");
                if shown.chars().count() > 40 {
                    format!("{}…", shown.chars().take(39).collect::<String>())
                } else {
                    shown
                }
            }
            Err(_) => text,
        },
        _ => text,
    }
}
