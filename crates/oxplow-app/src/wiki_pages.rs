//! Wiki-note disk sync + backlinks helpers.
//!
//! Bodies live as `.oxplow/wiki/<slug>.md`. The metadata row in
//! `wiki_page` is derived from the file (title, file refs, related
//! notes, body excerpt) by [`sync_from_disk`]. This module is the
//! pure parser + sync layer; the fs watcher in
//! [`crate::wiki_pages_watch`] drives it on file changes.
//!
//! Two ref shapes are extracted:
//!
//! 1. **`[[wikilinks]]`** — preferred form. The interior matches:
//!    - `path/with/slash.ext[:line]` → file ref
//!    - `bare-slug` (kebab-case, no slash, no extension) → related-note ref
//!
//!    Custom display text after `|` is stripped (`[[a/b.ts|label]]`).
//! 2. **Inline file paths** — a path written as plain text, without
//!    the `[[…]]` syntax. At least one slash + a 1-6 char extension,
//!    not preceded by `/` or alphanumerics so we don't pick up partial
//!    URLs.

use oxplow_domain::vocabulary::VocabularyHandle;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use oxplow_db::{Database, SqliteWikiPageStore};
use oxplow_domain::{DomainError, Timestamp};

/// One file reference parsed out of a wikilink: the path and an optional
/// line anchor. A version (`[[path@sha]]`) is not the page's to say —
/// freshness is the pin on the `page_ref` edge (`.context/knowledge.md`) —
/// so an `@…` segment is dropped (and stripped from bodies on write).
#[derive(Debug, Clone, PartialEq)]
pub struct WikiFileRef {
    pub path: String,
    pub line: Option<u32>,
}

/// Refs extracted from a note body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedRefs {
    /// Workspace-relative file paths (`src/foo.ts`).
    pub file_refs: Vec<String>,
    /// Workspace-relative directory paths (`src/components`). Source
    /// form in markdown is `[[dir:src/components]]` — the `dir:`
    /// prefix is the explicit directory marker (mirrors `git:` for
    /// commit refs). A trailing `/` on the path is tolerated and
    /// stripped.
    pub dir_refs: Vec<String>,
    /// Slugs of other wiki pages (`task-lifecycle`).
    pub related_notes: Vec<String>,
}

/// Parse a wikilink interior of the form `path[@version][:line]` into a
/// [`WikiFileRef`], or `None` if it doesn't look like a file path. The
/// `@version` segment is dropped (see [`WikiFileRef`]).
pub fn parse_wiki_file_ref(interior: &str) -> Option<WikiFileRef> {
    let trimmed = interior.trim();
    if trimmed.is_empty() {
        return None;
    }
    // The line anchor sits after the version: `path@<v>:42`. The version
    // may contain `:`, so split it off first.
    let path_and_line = match trimmed.split_once('@') {
        Some((path_part, version_part)) => match version_part.split_once(':') {
            Some((_, line)) => format!("{path_part}:{line}"),
            None => path_part.to_string(),
        },
        None => trimmed.to_string(),
    };
    let (bare, line) = match path_and_line.rsplit_once(':') {
        Some((p, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() => {
            (p.to_string(), l.parse::<u32>().ok())
        }
        _ => (path_and_line.clone(), None),
    };
    if bare.is_empty() || !looks_like_file(&bare) {
        return None;
    }
    Some(WikiFileRef { path: bare, line })
}

/// Parse `[[…]]` wikilinks + inline file paths out of `body`.
pub fn parse_refs(body: &str) -> ParsedRefs {
    if body.is_empty() {
        return ParsedRefs::default();
    }
    // Blank out code spans / fenced blocks first (shared with the
    // unified `refs::extract`) so illustrative `[[...]]` or path
    // tokens written inside backticks don't become refs.
    let masked = oxplow_domain::refs::mask_code_regions(body);
    let body = masked.as_str();
    let mut files = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    let mut notes = BTreeSet::new();
    // Detail entries preserve insertion order (first-seen wins on
    // duplicates) so the renderer can show them in author order.

    // 1. [[wikilinks]] first — they take priority, and we want to
    //    avoid double-counting an inline path that's also wrapped.
    for cap in find_wikilinks(body) {
        let interior = cap.split('|').next().unwrap_or(cap).trim();
        if interior.is_empty() {
            continue;
        }
        // Directory form. Directories don't carry @version yet; the
        // `dir:` prefix lives outside the path-and-anchor grammar.
        if let Some(dir) = looks_like_dir(interior) {
            dirs.insert(dir);
            continue;
        }
        // Try the rich file form first. `parse_wiki_file_ref` handles
        // `path@<version>[:line]` and bare `path[:line]`, returning
        // None if the interior doesn't shape like a file.
        if let Some(file) = parse_wiki_file_ref(interior) {
            files.insert(file.path);
            continue;
        }
        // Not a file — try slug form (`bare-slug`). Strip the line
        // anchor; slugs don't carry versions.
        let bare = interior.split(':').next().unwrap_or(interior);
        if !bare.is_empty() && looks_like_slug(bare) {
            notes.insert(bare.to_string());
        }
        // Drop git-commit refs (`[[abc1234]]` — 7-40 hex) silently;
        // they're for the renderer, not wiki indexing.
    }

    // 2. Inline file paths (plain-text mentions).
    let stripped = strip_urls(body);
    for path in find_inline_paths(&stripped) {
        files.insert(path);
    }

    ParsedRefs {
        file_refs: files.into_iter().collect(),
        dir_refs: dirs.into_iter().collect(),
        related_notes: notes.into_iter().collect(),
    }
}

/// The file's mtime as a `Timestamp`, when the platform exposes one.
fn file_mtime(path: &Path) -> Option<Timestamp> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(Timestamp::from_unix_ms(i64::try_from(ms).ok()?))
}

/// Restate `slug` from `.oxplow/wiki/<slug>.md` — how a hand edit (or
/// the boot scan) converges — through the same core as
/// `oxplow.knowledge.write_page` ([`crate::knowledge::write_page_tx`]), logged
/// as `system:wiki_watch`. Links aren't refused here: whatever is on
/// disk is recorded. A body with `@version` literals is written back
/// without them (versions live on the edges); an unchanged body (its
/// hash matches the row's, as after a command's own write) is a no-op;
/// a missing file deletes the page. `updated_at` is the file's mtime, so
/// a boot scan doesn't reset every page's recency. Whether anything
/// changed. A file whose name isn't a slug (`knowledge::valid_slug`, the
/// commands' rule) is not a page: nothing is recorded for it, and a row
/// it left behind is removed.
pub fn sync_page_tx(
    conn: &rusqlite::Connection,
    ev: &oxplow_db::EventCtx<'_>,
    project_dir: &Path,
    slug: &str,
) -> Result<bool, DomainError> {
    if !crate::knowledge::valid_slug(slug) {
        return logged_delete(conn, ev, slug);
    }
    let file_path = crate::knowledge::page_path(project_dir, slug);
    let raw = match fs::read_to_string(&file_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return logged_delete(conn, ev, slug);
        }
        Err(e) => return Err(DomainError::Storage(format!("read wiki page {slug}: {e}"))),
    };
    let body = strip_body_version_literals(&raw);
    if body != raw {
        if let Err(err) = fs::write(&file_path, &body) {
            tracing::warn!(
                slug,
                ?err,
                "failed to write back version-stripped wiki body"
            );
        }
    }
    let stored = oxplow_db::wiki_page_store::get_tx(conn, slug)?;
    if stored.is_some_and(|(_, hash)| hash == crate::knowledge::body_hash(&body)) {
        return Ok(false);
    }
    let (_, env) = crate::knowledge::write_page_tx(
        conn,
        ev,
        project_dir,
        &crate::knowledge::PageWrite {
            slug,
            body: &body,
            verified: &[],
            removed: &[],
            updated_at: file_mtime(&file_path).unwrap_or_else(Timestamp::now),
            anchors: oxplow_domain::Anchors::default(),
        },
    )?;
    ev.append(conn, &env)?;
    Ok(true)
}

/// Delete `slug`'s record as the watcher, logging it; whether there was a
/// page.
fn logged_delete(
    conn: &rusqlite::Connection,
    ev: &oxplow_db::EventCtx<'_>,
    slug: &str,
) -> Result<bool, DomainError> {
    match crate::knowledge::delete_page_tx(conn, ev, oxplow_domain::Anchors::default(), slug)? {
        Some(env) => {
            ev.append(conn, &env)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// [`sync_page_tx`] in its own transaction.
pub async fn sync_page(
    db: &Database,
    vocabulary: &VocabularyHandle,
    project_dir: &Path,
    slug: &str,
) -> Result<bool, DomainError> {
    let (vocabulary, project_dir, slug) = (
        vocabulary.clone(),
        project_dir.to_path_buf(),
        slug.to_string(),
    );
    db.transaction(move |tx| {
        let vocabulary = vocabulary.current();
        let ev = oxplow_db::EventCtx::system(&vocabulary, "wiki_watch");
        sync_page_tx(tx, &ev, &project_dir, &slug)
    })
    .await
}

/// True if `path` is a file located under any of `dirs` (a directory
/// ref the body cites). Workspace-relative, `/`-separated; an exact
/// equality is not a match (a file path can't equal a directory).
pub fn path_under_any_dir(path: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| {
        let prefix = format!("{}/", d.trim_end_matches('/'));
        path.starts_with(&prefix)
    })
}

/// Strip `@<version>` suffixes from `file:`, `dir:`, and bare
/// `[[path]]` wikilink forms. Removes `@disk` (tautology), `@local`,
/// and any author-supplied `@<sha>` / `@<branch>` literal — the
/// body is prose; version tracking is the page_ref row's job.
///
/// Hand-rolled string scanner (no regex dep). Scans for the two
/// shapes:
///
/// 1. `[[...]]` wikilinks — drop `@…` from inside the brackets,
///    preserving the optional `|label`.
/// 2. `(file:...)` / `(dir:...)` markdown URL forms — drop `@…`
///    between the path and the closing `)`.
///
/// Other parenthesized URL schemes (`http(s)`, `mailto:`,
/// `gitcommit:`) are left alone; `@` in those has its own meaning.
pub fn strip_body_version_literals(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        // [[...]] wikilink: handle as a unit
        if let Some(after_open) = rest.strip_prefix("[[") {
            if let Some(close) = after_open.find("]]") {
                let inner_end = 2 + close;
                let inner = &after_open[..close];
                let (path_part, label_part) = match inner.find('|') {
                    Some(p) => (&inner[..p], Some(&inner[p..])),
                    None => (inner, None),
                };
                let stripped_path = match path_part.find('@') {
                    Some(at) => &path_part[..at],
                    None => path_part,
                };
                out.push_str("[[");
                out.push_str(stripped_path);
                if let Some(label) = label_part {
                    out.push_str(label);
                }
                out.push_str("]]");
                i += inner_end + 2;
                continue;
            }
        }
        // (file:...) or (dir:...) markdown URL: handle as a unit
        if let Some(after_paren) = rest.strip_prefix('(') {
            let scheme_len = if after_paren.starts_with("file:") {
                Some(5usize)
            } else if after_paren.starts_with("dir:") {
                Some(4usize)
            } else {
                None
            };
            if let Some(scheme_len) = scheme_len {
                if let Some(close_rel) = after_paren.find(')') {
                    let url = &after_paren[..close_rel];
                    let path_after_scheme = &url[scheme_len..];
                    let stripped = match path_after_scheme.find('@') {
                        Some(at) => &path_after_scheme[..at],
                        None => path_after_scheme,
                    };
                    out.push('(');
                    out.push_str(&url[..scheme_len]);
                    out.push_str(stripped);
                    out.push(')');
                    // 1 for '(', `close_rel` chars of URL, 1 for ')'
                    i += 1 + close_rel + 1;
                    continue;
                }
            }
        }
        // Default: copy one UTF-8 char.
        let ch = rest.chars().next().expect("non-empty");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Retry an operation through transient `SQLITE_BUSY` errors with
/// exponential backoff (50/100/200/400ms, then one final attempt).
/// The common hit: the boot scan racing a previous process that's
/// still releasing the WAL lock — the per-project instance lock makes
/// real steady-state contention rare, so a short wait usually wins.
/// Non-`Busy` errors return immediately.
pub async fn retry_busy<T, F, Fut>(mut op: F) -> Result<T, DomainError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, DomainError>>,
{
    let mut delay = std::time::Duration::from_millis(50);
    for _ in 0..4 {
        match op().await {
            Err(DomainError::Busy(err)) => {
                tracing::debug!(%err, ?delay, "storage busy — retrying");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            other => return other,
        }
    }
    op().await
}

/// What a full scan accomplished: pages synced (or pruned) vs pages
/// that individually failed. The scan never aborts at the first bad
/// page — one poisoned file must not strand the rest of the index
/// stale.
#[derive(Debug, Default)]
pub struct ScanReport {
    pub synced: usize,
    pub failures: Vec<(String, DomainError)>,
}

/// Sync every `.md` file in the wiki dir and delete the pages whose file
/// is gone. Run once at watcher startup. Per-slug failures are collected
/// into the report (and warned) rather than aborting; transient `Busy`
/// errors retry with backoff first. The outer `Err` is reserved for
/// scan-fatal failures (listing the known rows).
pub async fn scan_and_sync_all(
    db: &Database,
    vocabulary: &VocabularyHandle,
    project_dir: &Path,
    store: &SqliteWikiPageStore,
) -> Result<ScanReport, DomainError> {
    let dir = wiki_pages_dir(project_dir);
    fs::create_dir_all(&dir).ok();
    let mut slugs: BTreeSet<String> = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
                slugs.insert(slug.to_string());
            }
        }
    }
    // A known page whose file is gone syncs to a delete.
    for page in retry_busy(|| store.list()).await? {
        slugs.insert(page.slug);
    }
    let mut report = ScanReport::default();
    for slug in &slugs {
        match retry_busy(|| sync_page(db, vocabulary, project_dir, slug)).await {
            Ok(_) => report.synced += 1,
            Err(err) => {
                tracing::warn!(slug, ?err, "wiki page sync failed during scan");
                report.failures.push((slug.clone(), err));
            }
        }
    }
    Ok(report)
}

pub fn wiki_pages_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".oxplow").join("wiki")
}

pub fn extract_title(body: &str, fallback: &str) -> String {
    for line in body.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Find every `[[…]]` interior. Naive scan; handles balanced pairs
/// only — we don't need nested wikilinks.
fn find_wikilinks(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find the closing `]]`.
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    if let Ok(interior) = std::str::from_utf8(&bytes[start..j]) {
                        out.push(interior);
                    }
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j + 1 >= bytes.len() {
                break;
            }
            continue;
        }
        i += 1;
    }
    out
}

/// If `s` is a directory wikilink target (`dir:<path>`), return the
/// stripped path. Otherwise return None. The `dir:` prefix is the
/// explicit directory marker — mirrors `git:` for commit refs.
fn looks_like_dir(s: &str) -> Option<String> {
    let trimmed = s.trim();
    let raw = trimmed.strip_prefix("dir:")?.trim_start();
    let bare = raw.trim_end_matches('/');
    if bare.is_empty() || bare.contains('\n') || bare.contains('|') {
        return None;
    }
    // Reject double-slash sequences (`//`), absolute paths, and URL
    // tails — directory refs are always workspace-relative.
    if bare.starts_with('/') || bare.contains("//") {
        return None;
    }
    Some(bare.to_string())
}

fn looks_like_file(s: &str) -> bool {
    if !s.contains('/') {
        return false;
    }
    // Trailing extension 1-6 chars after the last dot.
    if let Some(dot) = s.rfind('.') {
        let ext = &s[dot + 1..];
        if (1..=6).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return true;
        }
    }
    false
}

fn looks_like_slug(s: &str) -> bool {
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    if s.contains('/') || s.contains('.') || s.contains(' ') {
        return false;
    }
    // Skip git commit hashes (7-40 hex), they go to the renderer
    // not the wiki index.
    if matches!(s.len(), 7..=40) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn strip_urls(body: &str) -> String {
    // Replace URL-shaped runs with a space so the inline-path scan
    // doesn't pick up `https://example.com/path.json`. Cheap state
    // machine; full URL grammar isn't needed.
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `<scheme>://`.
        if i + 3 < bytes.len() && bytes[i..i + 3] == *b"://" {
            // Skip back to the start of the scheme word and forward
            // through the URL.
            // (We've already emitted the chars before `://`; rewind out.)
            for _ in 0..out
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphabetic() || matches!(*c, '+' | '-' | '.'))
                .count()
            {
                out.pop();
            }
            i += 3;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(' ');
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn find_inline_paths(body: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    // Tokenize on whitespace + a few punctuation chars; check each
    // token for "looks like path/to/file.ext".
    let separators: &[char] = &[
        ' ', '\t', '\n', '\r', ',', ';', '(', ')', '[', ']', '"', '\'',
    ];
    for token in body.split(|c: char| separators.contains(&c)) {
        // Trim *trailing* punctuation only (sentence periods, commas,
        // etc.). A leading `.` is significant — it marks a dotfile dir
        // like `.context/…` or `.github/…`; stripping it would forge a
        // phantom dot-less path that collides with nothing on disk.
        let trimmed = token.trim_end_matches(['.', ',', ';', ':']);
        if trimmed.is_empty() || trimmed.starts_with('/') {
            continue;
        }
        // Strip a trailing `:line` anchor (numeric only) so
        // `src/foo.rs:42` from a stack trace still parses as a file.
        // Wikilinks already accept this anchor; the inline scan needs
        // to match. Non-numeric anchors (e.g. `:fn_name`) are dropped
        // — `looks_like_file` will reject the polluted extension and
        // we don't try to recover a Symbol-style anchor here.
        let candidate: &str = if let Some((path_part, anchor)) = trimmed.rsplit_once(':') {
            if !anchor.is_empty() && anchor.chars().all(|c| c.is_ascii_digit()) {
                path_part
            } else {
                trimmed
            }
        } else {
            trimmed
        };
        if looks_like_file(candidate) && !candidate.starts_with("//") {
            out.insert(candidate.to_string());
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Core's vocabulary with oxplow's tasks as the work list (`tsk<n>`).
    fn tasks_vocabulary() -> oxplow_domain::vocabulary::VocabularyHandle {
        let mut v = oxplow_domain::vocabulary::Vocabulary::core();
        v.kinds = v.kinds.with_work_item_ids("oxplow", r"tsk\d+").unwrap();
        oxplow_domain::vocabulary::VocabularyHandle::new(v)
    }

    fn vocabulary() -> VocabularyHandle {
        tasks_vocabulary()
    }

    /// Re-syncing an unchanged file (e.g. the boot-time full scan on
    /// every daemon/app start) must not bump `updated_at` — the wiki
    /// index sorts and labels pages by it, and a boot that stamps
    /// "now" on every row destroys the recency signal.
    #[tokio::test]
    async fn resync_without_change_preserves_updated_at() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let wiki_dir = wiki_pages_dir(&project);
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(wiki_dir.join("note.md"), "# Note\nbody\n").unwrap();

        let db = oxplow_db::Database::in_memory();
        let store = oxplow_db::SqliteWikiPageStore::new(db.clone());

        sync_page(&db, &vocabulary(), &project, "note")
            .await
            .unwrap();
        let first = store.get("note").await.unwrap().unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        sync_page(&db, &vocabulary(), &project, "note")
            .await
            .unwrap();
        let second = store.get("note").await.unwrap().unwrap();
        assert_eq!(
            second.updated_at, first.updated_at,
            "unchanged resync must not bump updated_at"
        );

        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        std::fs::write(wiki_dir.join("note.md"), "# Note\nedited body\n").unwrap();
        sync_page(&db, &vocabulary(), &project, "note")
            .await
            .unwrap();
        let third = store.get("note").await.unwrap().unwrap();
        assert!(
            third.updated_at > first.updated_at,
            "content change must bump updated_at"
        );
    }

    /// A poisoned page (here: a *directory* named `bad.md`, which makes
    /// `read_to_string` fail) must not strand the rest of the wiki
    /// stale — the scan keeps going and reports the failure.
    #[tokio::test]
    async fn scan_continues_past_poisoned_page() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let wiki_dir = wiki_pages_dir(&project);
        std::fs::create_dir_all(wiki_dir.join("bad.md")).unwrap();
        std::fs::write(wiki_dir.join("good.md"), "# Good\nbody\n").unwrap();
        std::fs::write(wiki_dir.join("zz-later.md"), "# Later\nbody\n").unwrap();

        let db = oxplow_db::Database::in_memory();
        let store = oxplow_db::SqliteWikiPageStore::new(db.clone());

        let report = scan_and_sync_all(&db, &vocabulary(), &project, &store)
            .await
            .unwrap();
        assert_eq!(report.synced, 2, "both healthy pages sync");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].0, "bad");
        assert!(store.get("good").await.unwrap().is_some());
        assert!(
            store.get("zz-later").await.unwrap().is_some(),
            "slug sorting after the poisoned one must still sync"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retry_busy_retries_transient_busy_then_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let out: Result<u32, DomainError> = retry_busy(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(DomainError::Busy("database is locked".into()))
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_busy_gives_up_after_backoff_and_surfaces_busy() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let out: Result<(), DomainError> = retry_busy(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(DomainError::Busy("database is locked".into())) }
        })
        .await;
        assert!(matches!(out, Err(DomainError::Busy(_))));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            5,
            "4 backoff retries + final"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retry_busy_does_not_retry_non_busy_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let out: Result<(), DomainError> = retry_busy(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(DomainError::Storage("disk on fire".into())) }
        })
        .await;
        assert!(matches!(out, Err(DomainError::Storage(_))));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn strip_version_drops_disk_and_explicit_pins_from_wikilinks() {
        assert_eq!(
            strip_body_version_literals("see [[src/foo.ts@disk]]"),
            "see [[src/foo.ts]]"
        );
        assert_eq!(
            strip_body_version_literals("see [[src/foo.ts@abc1234]]"),
            "see [[src/foo.ts]]"
        );
        assert_eq!(
            strip_body_version_literals("[[src/foo.ts@disk|the helper]]"),
            "[[src/foo.ts|the helper]]"
        );
        // Bare wikilink untouched.
        assert_eq!(
            strip_body_version_literals("[[src/foo.ts]]"),
            "[[src/foo.ts]]"
        );
    }

    #[test]
    fn strip_version_drops_pins_from_markdown_urls() {
        assert_eq!(
            strip_body_version_literals("([foo](file:src/foo.ts@disk))"),
            "([foo](file:src/foo.ts))"
        );
        assert_eq!(
            strip_body_version_literals("([dir](dir:src/components@abc))"),
            "([dir](dir:src/components))"
        );
        // http URLs with @ are left alone (legitimate userinfo).
        assert_eq!(
            strip_body_version_literals("(https://user@host/path)"),
            "(https://user@host/path)"
        );
        // gitcommit shas survive (no @ to strip — but defense in
        // depth: the scanner only fires on file:/dir: prefixes).
        assert_eq!(
            strip_body_version_literals("([abc](gitcommit:abc1234))"),
            "([abc](gitcommit:abc1234))"
        );
    }

    #[test]
    fn strip_version_preserves_utf8() {
        // Em dash and accented chars survive the byte-aware scanner.
        let body = "Look — [[résumé.md@disk]] — okay?";
        assert_eq!(
            strip_body_version_literals(body),
            "Look — [[résumé.md]] — okay?"
        );
    }

    #[test]
    fn parse_extracts_wikilink_files() {
        let refs = parse_refs("see [[src/foo.ts]] and [[src/bar.tsx:42]]");
        assert_eq!(refs.file_refs, vec!["src/bar.tsx", "src/foo.ts"]);
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn dotfile_dir_wikilink_keeps_leading_dot_and_does_not_duplicate() {
        // Regression: a `[[.context/foo.md]]` wikilink must yield
        // exactly one file ref WITH its leading dot. The inline-path
        // fallback used to trim the leading `.` (treating it like
        // trailing sentence punctuation), producing a phantom
        // dot-stripped `context/foo.md` alongside the real ref.
        let refs = parse_refs("see [[.context/data-model.md]] for the schema");
        assert_eq!(refs.file_refs, vec![".context/data-model.md"]);
    }

    #[test]
    fn inline_dotfile_path_keeps_leading_dot() {
        // A bare inline dotfile path (no wikilink) must also keep its
        // leading dot.
        let refs = parse_refs("edit .github/workflows/ci.yml to fix CI");
        assert_eq!(refs.file_refs, vec![".github/workflows/ci.yml"]);
    }

    #[test]
    fn inline_path_still_strips_trailing_sentence_period() {
        // The trailing-punctuation trim must still work: a path that
        // ends a sentence keeps no trailing dot.
        let refs = parse_refs("we touched src/lib.rs.");
        assert_eq!(refs.file_refs, vec!["src/lib.rs"]);
    }

    #[test]
    fn parse_ignores_links_inside_inline_code() {
        // Regression: `` `[[path]]` `` in a freshness note created a
        // phantom "path" related-note. Code spans must be opaque.
        let refs = parse_refs("normalized to the bare `[[path]]` form, e.g. `src/foo.rs`");
        assert!(
            refs.related_notes.is_empty(),
            "got {:?}",
            refs.related_notes
        );
        assert!(refs.file_refs.is_empty(), "got {:?}", refs.file_refs);
    }

    #[test]
    fn parse_ignores_links_inside_fenced_block() {
        let refs = parse_refs("text\n```\n[[in-fence]] [[src/x.rs]]\n```\nkeep [[real-slug]]");
        assert_eq!(refs.related_notes, vec!["real-slug"]);
        assert!(refs.file_refs.is_empty(), "got {:?}", refs.file_refs);
    }

    #[test]
    fn parse_extracts_wikilink_notes() {
        let refs = parse_refs("related: [[task-lifecycle]] and [[stop-hook-pipeline]]");
        assert_eq!(
            refs.related_notes,
            vec!["stop-hook-pipeline", "task-lifecycle"]
        );
    }

    #[test]
    fn parse_strips_display_text() {
        let refs = parse_refs("[[src/foo.ts|the foo helper]]");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
    }

    #[test]
    fn parse_extracts_directory_refs() {
        let refs = parse_refs("see [[dir:src/components]] for the buttons");
        assert_eq!(refs.dir_refs, vec!["src/components"]);
        assert!(refs.file_refs.is_empty());
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn parse_directory_ref_with_label() {
        let refs = parse_refs("[[dir:src/components|the components folder]]");
        assert_eq!(refs.dir_refs, vec!["src/components"]);
    }

    #[test]
    fn parse_directory_ref_tolerates_trailing_slash() {
        let refs = parse_refs("[[dir:src/foo/]]");
        assert_eq!(refs.dir_refs, vec!["src/foo"]);
    }

    #[test]
    fn parse_directory_ref_dedupes() {
        let refs = parse_refs("[[dir:src/foo]] then [[dir:src/foo]] and [[dir:src/bar/baz]]");
        assert_eq!(refs.dir_refs, vec!["src/bar/baz", "src/foo"]);
    }

    #[test]
    fn parse_keeps_file_form_without_dir_prefix() {
        // Regression: `[[src/foo.ts]]` must remain a file ref even
        // after the directory branch was added.
        let refs = parse_refs("[[src/foo.ts]]");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
        assert!(refs.dir_refs.is_empty());
    }

    #[test]
    fn parse_skips_commit_hashes() {
        let refs = parse_refs("see [[abc1234]] and [[abc1234567890ab]]");
        assert!(refs.file_refs.is_empty());
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn parse_picks_up_inline_paths() {
        let refs = parse_refs("the file src/foo.ts has the bug");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
    }

    #[test]
    fn parse_skips_urls() {
        let refs = parse_refs("see https://example.com/path.json for details");
        assert!(refs.file_refs.is_empty());
    }

    #[test]
    fn a_wikilinks_version_is_dropped_and_its_line_kept() {
        // Freshness is the edge's pin, not the link's: any `@…` segment
        // names the same file as the bare path.
        let refs = parse_refs(
            "see [[src/foo.ts@HEAD]] and [[src/foo.ts@disk]] and [[src/foo.ts@abc1234:42]]",
        );
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
        assert_eq!(
            parse_wiki_file_ref("src/foo.ts@HEAD:42"),
            Some(WikiFileRef {
                path: "src/foo.ts".into(),
                line: Some(42)
            })
        );
        assert_eq!(
            parse_wiki_file_ref("src/foo.ts:7"),
            Some(WikiFileRef {
                path: "src/foo.ts".into(),
                line: Some(7)
            })
        );
        assert_eq!(parse_wiki_file_ref("src/foo.rs@").unwrap().line, None);
    }

    #[test]
    fn extract_title_picks_first_h1() {
        assert_eq!(extract_title("# Hello\n\nbody", "fallback"), "Hello");
        assert_eq!(extract_title("no heading", "fallback"), "fallback");
    }

    // ---- Edge cases: looks_like_file ----

    #[test]
    fn looks_like_file_requires_slash_and_extension() {
        assert!(looks_like_file("src/foo.rs"));
        assert!(looks_like_file("a/b/c.tsx"));
        assert!(looks_like_file("docs/README.md"));
        assert!(!looks_like_file("foo.rs")); // no slash
        assert!(!looks_like_file("src/foo")); // no extension
        assert!(!looks_like_file("")); // empty
    }

    #[test]
    fn looks_like_file_rejects_non_alphanumeric_extension() {
        // Trailing colon polluted the extension check before the
        // line-anchor strip in find_inline_paths landed.
        assert!(!looks_like_file("src/foo.rs:42"));
        assert!(!looks_like_file("src/foo.r$"));
    }

    #[test]
    fn looks_like_file_extension_length_bounds() {
        assert!(looks_like_file("a/b.x")); // 1 char ext
        assert!(looks_like_file("a/b.abcdef")); // 6 char ext
        assert!(!looks_like_file("a/b.abcdefg")); // 7 char ext rejected
    }

    // ---- Edge cases: looks_like_dir ----

    #[test]
    fn looks_like_dir_strips_prefix_and_trailing_slash() {
        assert_eq!(
            looks_like_dir("dir:src/components"),
            Some("src/components".into())
        );
        assert_eq!(looks_like_dir("dir:src/foo/"), Some("src/foo".into()));
    }

    #[test]
    fn looks_like_dir_rejects_absolute_and_double_slash() {
        assert_eq!(looks_like_dir("dir:/abs"), None);
        assert_eq!(looks_like_dir("dir:src//double"), None);
    }

    #[test]
    fn looks_like_dir_rejects_empty_and_missing_prefix() {
        assert_eq!(looks_like_dir("dir:"), None);
        assert_eq!(looks_like_dir("src/components"), None); // no prefix
    }

    // ---- Edge cases: looks_like_slug ----

    #[test]
    fn looks_like_slug_accepts_kebab_and_underscore() {
        assert!(looks_like_slug("task-lifecycle"));
        assert!(looks_like_slug("snake_case_slug"));
        assert!(looks_like_slug("a"));
    }

    #[test]
    fn looks_like_slug_rejects_dotted_and_pathy() {
        assert!(!looks_like_slug("foo.bar"));
        assert!(!looks_like_slug("foo/bar"));
        assert!(!looks_like_slug("foo bar"));
        assert!(!looks_like_slug(""));
    }

    #[test]
    fn looks_like_slug_rejects_commit_hash_shape() {
        // 7-40 char all-hex strings look like git shas; the wiki
        // doesn't index them.
        assert!(!looks_like_slug("abc1234"));
        assert!(!looks_like_slug("abc1234567890ab"));
        // 6-char and 41-char hex strings are NOT rejected (just
        // outside the commit-hash heuristic window).
        assert!(looks_like_slug("abcdef"));
    }

    #[test]
    fn looks_like_slug_too_long_rejected() {
        let s = "a".repeat(81);
        assert!(!looks_like_slug(&s));
    }

    // ---- Edge cases: parse_wiki_file_ref ----

    #[test]
    fn parse_wiki_file_ref_returns_none_for_non_path() {
        assert!(parse_wiki_file_ref("").is_none());
        assert!(parse_wiki_file_ref("just-a-slug").is_none());
        assert!(parse_wiki_file_ref("nodot/path").is_none());
    }

    #[test]
    fn parse_wiki_file_ref_strips_line_only_when_all_digits() {
        let r = parse_wiki_file_ref("src/foo.rs:42").unwrap();
        assert_eq!(r.path, "src/foo.rs");
        assert_eq!(r.line, Some(42));
        // Non-numeric anchor is rejected (we don't try to recover
        // by stripping it — the user has to write `:N` or omit it).
        assert!(parse_wiki_file_ref("src/foo.rs:fn_name").is_none());
    }

    // ---- TDD: inline paths must accept :line anchors ----
    //
    // Prior to this commit, find_inline_paths called looks_like_file
    // on the raw token, so `src/foo.rs:42` failed the extension check
    // (ext became "rs:42") and was silently dropped. Wikilinks like
    // `[[src/foo.rs:42]]` already supported the :line anchor — the
    // inline scan didn't, so a stack-trace-style mention couldn't be
    // backlinked.

    #[test]
    fn parse_picks_up_inline_path_with_line_anchor() {
        let refs = parse_refs("error at src/foo.rs:42 in the trace");
        assert_eq!(refs.file_refs, vec!["src/foo.rs"]);
    }

    #[test]
    fn parse_picks_up_inline_path_with_multi_digit_line() {
        let refs = parse_refs("see crates/oxplow-app/src/lib.rs:1234");
        assert_eq!(refs.file_refs, vec!["crates/oxplow-app/src/lib.rs"]);
    }

    #[test]
    fn parse_inline_path_rejects_non_numeric_anchor() {
        // Symbol-anchor (`:fn_name`) is not a line anchor — keep the
        // current "ignore" behavior so we don't accidentally index
        // `foo.rs:bar` as a file `foo.rs`. The inline scan only
        // recognizes numeric line anchors.
        let refs = parse_refs("see src/foo.rs:fn_name in the impl");
        assert!(refs.file_refs.is_empty());
    }

    #[test]
    fn parse_url_immediately_after_path_does_not_eat_path() {
        // Defends against the strip_urls rewinder over-popping past
        // a file path. With a separating space the URL strip is
        // straightforward; the file path survives.
        let refs = parse_refs("see src/foo.rs https://example.com/x.html");
        assert_eq!(refs.file_refs, vec!["src/foo.rs"]);
    }

    // ---- Edge cases: extract_title ----

    #[test]
    fn extract_title_skips_empty_h1() {
        // A `# ` with nothing after isn't a useful title; fall through
        // to the next line / fallback.
        assert_eq!(
            extract_title("#  \n# Real Title\n", "fallback"),
            "Real Title"
        );
    }

    #[test]
    fn extract_title_tolerates_leading_whitespace() {
        assert_eq!(extract_title("   # Indented\n", "fb"), "Indented");
    }

    #[test]
    fn extract_title_ignores_h2_and_lower() {
        // Only `# ` (h1) counts; `## h2` is body content.
        assert_eq!(extract_title("## H2\n# H1\n", "fb"), "H1");
    }

    // ---- Edge cases: find_wikilinks ----

    #[test]
    fn find_wikilinks_unclosed_bracket_does_not_panic() {
        // `[[unclosed` — the byte scanner should walk to the end and
        // emit nothing, not panic on the truncated buffer.
        let v = find_wikilinks("text [[unclosed and more text");
        assert!(v.is_empty());
    }

    #[test]
    fn find_wikilinks_empty_pair_emits_empty_interior() {
        let v = find_wikilinks("[[]]");
        assert_eq!(v, vec![""]);
    }

    #[test]
    fn find_wikilinks_handles_unicode_around() {
        // The scanner indexes by bytes; UTF-8 multi-byte chars
        // outside the brackets must not break the match.
        let v = find_wikilinks("café [[src/foo.rs]] résumé");
        assert_eq!(v, vec!["src/foo.rs"]);
    }

    // ---- Unified page_ref projection ----

    /// End-to-end: write a wiki body that mentions `wi-1` and a file,
    /// run the disk sync with the page-ref store attached, and verify
    /// that `list_backlinks(task, wi-1)` and
    /// `list_backlinks(file, …)` both return the wiki page as a
    /// source. This is the user-visible promise: every page kind that
    /// gets mentioned in a wiki body shows the wiki in its backlinks.
    #[tokio::test]
    async fn wiki_sync_projects_unified_backlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path();
        std::fs::create_dir_all(wiki_pages_dir(project)).unwrap();
        std::fs::write(
            wiki_pages_dir(project).join("intro.md"),
            "# Intro\nblocks [[tsk1]] and touches [[src/app.rs]] and finding:fnd-1\n",
        )
        .unwrap();

        let db = oxplow_db::Database::in_memory();
        let page_refs = oxplow_db::SqlitePageRefStore::new(db.clone());

        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();

        // wi-1 backlink picks up the wiki source.
        let inbound_wi = page_refs
            .list_backlinks("work_item", "oxplow:tsk1", None)
            .await
            .unwrap();
        assert_eq!(inbound_wi.len(), 1);
        assert_eq!(inbound_wi[0].source_kind, "wiki");
        assert_eq!(inbound_wi[0].source_id, "intro");

        // file backlink also points at the wiki.
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(
            inbound_file.iter().any(|e| e.source_id == "intro"),
            "expected wiki:intro in file backlinks; got {inbound_file:?}"
        );

        // finding backlink works too.
        let inbound_finding = page_refs
            .list_backlinks("finding", "fnd-1", None)
            .await
            .unwrap();
        assert!(inbound_finding.iter().any(|e| e.source_id == "intro"));

        // Outbound view of the wiki shows the same edges.
        let outbound = page_refs
            .list_outbound("wiki", "intro", None)
            .await
            .unwrap();
        let targets: std::collections::BTreeSet<_> = outbound
            .iter()
            .map(|e| (e.target_kind.as_str(), e.target_id.as_str()))
            .collect();
        assert!(targets.contains(&("work_item", "oxplow:tsk1")));
        assert!(targets.contains(&("file", "src/app.rs")));
        assert!(targets.contains(&("finding", "fnd-1")));
    }

    #[test]
    fn path_under_any_dir_matches_only_descendants() {
        let dirs = vec!["crates/oxplow-control-plane".to_string()];
        assert!(path_under_any_dir(
            "crates/oxplow-control-plane/src/lib.rs",
            &dirs
        ));
        // The directory itself is not "under" itself.
        assert!(!path_under_any_dir("crates/oxplow-control-plane", &dirs));
        // A sibling that merely shares a prefix is not under it.
        assert!(!path_under_any_dir(
            "crates/oxplow-control-plane-x/y.rs",
            &dirs
        ));
        assert!(!path_under_any_dir("crates/other/z.rs", &dirs));
    }

    #[tokio::test]
    async fn wiki_sync_preserves_verification_edge_under_cited_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path();
        std::fs::create_dir_all(wiki_pages_dir(project)).unwrap();
        let body_path = wiki_pages_dir(project).join("intro.md");
        // Body cites the DIRECTORY only, not the file.
        std::fs::write(&body_path, "see [[dir:crates/cp]] for details").unwrap();

        let db = oxplow_db::Database::in_memory();
        let page_refs = oxplow_db::SqlitePageRefStore::new(db.clone());

        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();
        // Materialize a verification edge for a file under the cited dir.
        page_refs
            .upsert_edge(
                oxplow_db::PageRefEdge::new(
                    "wiki",
                    "intro",
                    "file",
                    "crates/cp/src/lib.rs",
                    "wiki_file_ref",
                )
                .with_version(42, None, false),
            )
            .await
            .unwrap();

        // Re-sync with the body UNCHANGED: the verification edge must
        // survive (and keep its pin), even though it isn't in the body.
        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();
        let after = page_refs
            .list_backlinks("file", "crates/cp/src/lib.rs", None)
            .await
            .unwrap();
        assert_eq!(after.len(), 1, "verification edge should survive re-sync");
        assert_eq!(after[0].local_snapshot_id, Some(42), "pin preserved");

        // Remove the dir ref from the body → next sync prunes the now-
        // orphaned verification edge.
        std::fs::write(&body_path, "no refs at all now").unwrap();
        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();
        let gone = page_refs
            .list_backlinks("file", "crates/cp/src/lib.rs", None)
            .await
            .unwrap();
        assert!(
            gone.is_empty(),
            "verification edge should self-clean once its dir ref is gone, got {gone:?}"
        );
    }

    /// When a wiki body changes to remove a ref, the next sync must
    /// drop the corresponding backlink edge.
    #[tokio::test]
    async fn wiki_sync_replaces_old_edges() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path();
        std::fs::create_dir_all(wiki_pages_dir(project)).unwrap();
        let body_path = wiki_pages_dir(project).join("intro.md");
        std::fs::write(&body_path, "[[tsk1]] [[tsk2]]").unwrap();

        let db = oxplow_db::Database::in_memory();
        let page_refs = oxplow_db::SqlitePageRefStore::new(db.clone());

        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();
        // Now drop wi-2 from the body.
        std::fs::write(&body_path, "[[tsk1]] only").unwrap();
        sync_page(&db, &vocabulary(), project, "intro")
            .await
            .unwrap();

        let inbound_2 = page_refs
            .list_backlinks("work_item", "oxplow:tsk2", None)
            .await
            .unwrap();
        assert!(inbound_2.is_empty(), "expected no backlinks after removal");
        let inbound_1 = page_refs
            .list_backlinks("work_item", "oxplow:tsk1", None)
            .await
            .unwrap();
        assert_eq!(inbound_1.len(), 1);
    }
}
