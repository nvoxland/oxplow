//! The `content` loader: what the system hands a resource page (a page
//! with `for:`) of the resource it shows. The page never queries for what
//! it renders; it reads the one row of `content`:
//!
//! - a **file** (`oxplow/file`): its filesystem's `stat` — `uri, path,
//!   filesystem, rev, kind, size, modified_at, is_text, language` — and,
//!   when the page asks for text (`content: { text: true }`), `text` and
//!   `truncated`. Text is read in a ranged read of `max_bytes + 1`, so a
//!   big file costs no more than a small one; a binary file has `NULL`
//!   text. No such file: no row.
//! - a resource a **dataset** resolves (its declaration's `resolve:`; a
//!   commit, a work item, a metric): that dataset's row, every column.
//! - any other URI (a folder, a web page): the one row `{ uri }`.
//!
//! See `.context/pages-and-tabs.md` "Resource pages and standalone pages".

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::filesystem::{ByteRange, FsCx, FsKind};
use oxplow_domain::uri::resources as r;
use oxplow_domain::uri::OxplowUri;
use oxplow_domain::StreamId;

use crate::Services;

/// How a page's `content` loader reads (`content:` in the page file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentOpts {
    /// Read a file's text too.
    pub text: bool,
    /// The most bytes of text read.
    pub max_bytes: u32,
}

/// The record `uri` names, for a viewer in `stream`.
pub async fn resolve_content(
    svc: &Services,
    uri: &str,
    stream: Option<i64>,
    opts: ContentOpts,
) -> Result<SqlQueryResult, String> {
    // Not an oxplow URI (a web page's): all there is is the URI.
    let Ok(parsed) = OxplowUri::parse(uri.trim()) else {
        return Ok(uri_row(uri.trim()));
    };
    let identity = parsed.identity();
    if parsed.resource().to_string() == r::FILE {
        return file_row(svc, &parsed, &identity, stream.map(StreamId::new), opts).await;
    }
    let dataset = svc
        .vocabulary
        .current()
        .resources
        .resolving_dataset(parsed.resource())
        .map(str::to_string);
    match dataset {
        Some(view) => svc
            .sql
            .query_sql(
                &format!(
                    "SELECT * FROM \"{}\" WHERE uri = ?1 LIMIT 1",
                    view.replace('"', "")
                ),
                vec![SqlCell::Text(identity)],
                Some(1),
            )
            .await
            .map_err(|e| e.to_string()),
        None => Ok(uri_row(&identity)),
    }
}

fn uri_row(uri: &str) -> SqlQueryResult {
    table(vec!["uri".into()], vec![vec![SqlCell::Text(uri.into())]])
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

fn opt<T>(value: Option<T>, cell: impl FnOnce(T) -> SqlCell) -> SqlCell {
    value.map(cell).unwrap_or(SqlCell::Null(()))
}

async fn file_row(
    svc: &Services,
    parsed: &OxplowUri,
    identity: &str,
    stream: Option<StreamId>,
    opts: ContentOpts,
) -> Result<SqlQueryResult, String> {
    let fs = svc
        .filesystems
        .resolve(parsed.param("filesystem"))
        .map_err(|e| e.to_string())?;
    let cx = FsCx { stream };
    let rev = parsed.param("rev");
    let path = parsed.param("path").unwrap_or_default();
    let mut columns: Vec<String> = [
        "uri",
        "path",
        "filesystem",
        "rev",
        "kind",
        "size",
        "modified_at",
        "is_text",
        "language",
    ]
    .map(String::from)
    .into();
    if opts.text {
        columns.extend(["text".to_string(), "truncated".to_string()]);
    }
    let mut out = table(columns, Vec::new());
    // A change to the files can change the row: the page runs again.
    out.reads.datasets = vec!["v_file".to_string()];
    let Some(stat) = fs.stat(&cx, rev, path).await.map_err(|e| e.to_string())? else {
        return Ok(out);
    };
    let mut row = vec![
        SqlCell::Text(identity.into()),
        SqlCell::Text(path.into()),
        SqlCell::Text(fs.id().into()),
        opt(rev, |r| SqlCell::Text(r.into())),
        SqlCell::Text(
            match stat.kind {
                FsKind::File => "file",
                FsKind::Dir => "dir",
            }
            .into(),
        ),
        opt(stat.size, |s| SqlCell::Int(s as i64)),
        opt(stat.modified_at, SqlCell::Int),
        opt(stat.is_text, SqlCell::Bool),
        opt(stat.language, SqlCell::Text),
    ];
    if opts.text {
        let (text, truncated) = if stat.is_text == Some(true) {
            let max = opts.max_bytes as usize;
            let range = ByteRange {
                offset: 0,
                len: max as u64 + 1,
            };
            let bytes = fs
                .read(&cx, rev, path, Some(range))
                .await
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            let truncated = bytes.len() > max;
            if truncated {
                let mut cut = max.min(text.len());
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            (SqlCell::Text(text), truncated)
        } else {
            (SqlCell::Null(()), false)
        };
        row.extend([text, SqlCell::Bool(truncated)]);
    }
    out.rows.push(row);
    Ok(out)
}
