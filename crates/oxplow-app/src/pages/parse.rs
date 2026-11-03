//! The `parse` loader: one text read as rows (`.context/extensions-pages.md`
//! "Loaders"). A CSV or TSV is a table: the first record names the columns
//! (a blank header is `column_<n>`, a repeat `<name>_<k>`), every cell is
//! text, a short record is padded with nulls and a long one cut. A JSON or
//! YAML document is a tree, one row per value in document order: `id` (its
//! JSON pointer), `parent` (the pointer of the value holding it; none at the
//! top), `name` (its key or index), `kind` (`object`, `array`, `string`,
//! `number`, `boolean`, `null`), `value` (a scalar's text) and `count` (a
//! container's children). The rows feed a `table` or a `tree` viz as any
//! loader's do; the result budget bounds them as it bounds every loader.

use oxplow_db::{SqlCell, SqlQueryResult};

use super::schema::ParseFormat;

/// `text` as rows, read as `format`; a text that isn't the format is the
/// loader's error.
pub fn parse_text(format: ParseFormat, text: &str) -> Result<SqlQueryResult, String> {
    match format {
        ParseFormat::Csv => delimited(text, b','),
        ParseFormat::Tsv => delimited(text, b'\t'),
        ParseFormat::Json => {
            tree(serde_json::from_str(text).map_err(|e| format!("the text isn't json: {e}"))?)
        }
        ParseFormat::Yaml => {
            tree(serde_yaml::from_str(text).map_err(|e| format!("the text isn't yaml: {e}"))?)
        }
    }
}

fn table(columns: Vec<String>, rows: Vec<Vec<SqlCell>>) -> SqlQueryResult {
    SqlQueryResult {
        columns,
        rows,
        truncated: false,
        reads: oxplow_db::Reads::default(),
        freshness: Vec::new(),
        generation: None,
        coverage: Vec::new(),
    }
}

fn delimited(text: &str, delimiter: u8) -> Result<SqlQueryResult, String> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| format!("the text isn't delimited rows: {e}"))?
        .clone();
    let columns = column_names(headers.iter());
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("the text isn't delimited rows: {e}"))?;
        let mut row: Vec<SqlCell> = record
            .iter()
            .take(columns.len())
            .map(|cell| SqlCell::Text(cell.to_string()))
            .collect();
        row.resize(columns.len(), SqlCell::Null(()));
        rows.push(row);
    }
    Ok(table(columns, rows))
}

/// Column names from a header: trimmed, a blank one named by its place, a
/// repeat numbered.
fn column_names<'a>(headers: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, header) in headers.enumerate() {
        let base = match header.trim() {
            "" => format!("column_{}", i + 1),
            h => h.to_string(),
        };
        let mut name = base.clone();
        let mut k = 1;
        while out.contains(&name) {
            k += 1;
            name = format!("{base}_{k}");
        }
        out.push(name);
    }
    out
}

const TREE_COLUMNS: [&str; 6] = ["id", "parent", "name", "kind", "value", "count"];

fn tree(doc: serde_json::Value) -> Result<SqlQueryResult, String> {
    let mut rows = Vec::new();
    match &doc {
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            children(&doc, "", &mut rows);
        }
        scalar => rows.push(row("/", None, None, scalar)),
    }
    Ok(table(TREE_COLUMNS.map(String::from).to_vec(), rows))
}

/// The rows of `holder`'s children, each followed by its own, in order.
fn children(holder: &serde_json::Value, at: &str, rows: &mut Vec<Vec<SqlCell>>) {
    let parent = (!at.is_empty()).then(|| at.to_string());
    let entries: Vec<(String, &serde_json::Value)> = match holder {
        serde_json::Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        serde_json::Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    };
    for (name, value) in entries {
        let id = format!("{at}/{}", pointer_token(&name));
        rows.push(row(&id, parent.clone(), Some(name), value));
        children(value, &id, rows);
    }
}

/// A key as a JSON pointer token (`~` and `/` escaped).
fn pointer_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn row(
    id: &str,
    parent: Option<String>,
    name: Option<String>,
    value: &serde_json::Value,
) -> Vec<SqlCell> {
    use serde_json::Value as V;
    let text = |s: Option<String>| s.map(SqlCell::Text).unwrap_or(SqlCell::Null(()));
    let (kind, shown, count) = match value {
        V::Object(map) => ("object", None, Some(map.len())),
        V::Array(items) => ("array", None, Some(items.len())),
        V::String(s) => ("string", Some(s.clone()), None),
        V::Number(n) => ("number", Some(n.to_string()), None),
        V::Bool(b) => ("boolean", Some(b.to_string()), None),
        V::Null => ("null", None, None),
    };
    vec![
        SqlCell::Text(id.to_string()),
        text(parent),
        text(name),
        SqlCell::Text(kind.to_string()),
        text(shown),
        count
            .map(|n| SqlCell::Int(n as i64))
            .unwrap_or(SqlCell::Null(())),
    ]
}
