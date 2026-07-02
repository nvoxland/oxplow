//! Pure cross-kind reference extractor.
//!
//! Given a free-text body (wiki page, task description, commit
//! message, task-note, …) extracts the references it makes to other
//! pages: file paths, directory paths, wiki slugs, task ids
//! (`tsk<n>`), finding ids (`finding:…`), and git commit shas.
//!
//! Lives in `oxplow-domain` because it has zero IO and no async — it
//! takes a `&str` and returns plain data. Every writer that mirrors
//! a body's outbound refs into the `page_ref` table calls this.
//!
//! Two ref shapes are recognised:
//!
//! 1. **`[[wikilinks]]`** — preferred form. The interior matches:
//!    - `dir:<path>` → directory ref
//!    - `git:<sha>` → commit ref (also `[[<sha>]]` if 7-40 hex)
//!    - `finding:<id>` → finding ref
//!    - `tsk<digits>` → task ref
//!    - `path/with/slash.ext[@version][:line]` → file ref
//!    - `bare-slug` (kebab-case, no slash, no extension) → wiki slug
//!
//!    Custom display text after `|` is stripped (`[[a/b.ts|label]]`).
//!
//! 2. **Inline mentions** — fallback for free-text:
//!    - bare `tsk<digits>` task ids
//!    - bare `finding:<id>` (with the prefix to disambiguate from words)
//!    - bare file-shaped paths (slash + 1–6 char extension)
//!
//! URLs are stripped before the inline scan so
//! `https://example.com/path.json` doesn't masquerade as a file ref.

use std::collections::BTreeSet;

/// Tree version a file ref is pinned to. `Disk` = working tree (the
/// default, also `@disk` / `@local` literal). `Ref(s)` = a git ref
/// (sha / branch / tag / `HEAD`) authored as `@<spec>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefVersion {
    Disk,
    Ref(String),
}

/// A file reference with optional version pin and line anchor, as
/// authored in the source body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRefDetail {
    pub path: String,
    pub version: RefVersion,
    pub line: Option<u32>,
}

/// All cross-kind references found in a body. Each list is
/// deduplicated; file refs preserve insertion order in
/// `files_detail` so the renderer can show them in author order
/// while `files` is the version-stripped path-only set used for
/// backlinks lookup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedRefs {
    pub files: Vec<String>,
    pub files_detail: Vec<FileRefDetail>,
    pub dirs: Vec<String>,
    pub wikis: Vec<String>,
    pub tasks: Vec<i64>,
    pub findings: Vec<String>,
    pub commits: Vec<String>,
}

/// A single classified reference — the recognized kind + payload of one
/// `[[…]]` wikilink interior. The typed counterpart of the buckets in
/// [`ExtractedRefs`]; used by the link checker to existence-check each
/// recognized target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    File(FileRefDetail),
    Dir(String),
    Wiki(String),
    Task(i64),
    Finding(String),
    Commit(String),
}

/// One `[[…]]` wikilink found in a body: its raw interior (the target,
/// before any `|label`) and the reference it resolves to — or `None`
/// when the interior matches no known ref shape (e.g. `#13`, the GitHub
/// form). The link checker turns `reference: None` into an
/// "unrecognized reference" warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedWikilink {
    pub raw: String,
    pub reference: Option<Reference>,
}

/// Classify a single `[[…]]` interior (already `|`-split and trimmed)
/// into its recognized [`Reference`], or `None` if it matches no known
/// shape. The single source of truth for what counts as a valid ref —
/// shared by [`extract`] and [`classify_wikilinks`] so the graph writer
/// and the link checker never drift.
fn classify_interior(interior: &str) -> Option<Reference> {
    if interior.is_empty() {
        return None;
    }
    // dir:<path>
    if let Some(rest) = strip_prefix_ci(interior, "dir:") {
        return clean_dir(rest).map(Reference::Dir);
    }
    // git:<sha>
    if let Some(rest) = strip_prefix_ci(interior, "git:") {
        return clean_commit(rest).map(Reference::Commit);
    }
    // finding:<id>
    if let Some(rest) = strip_prefix_ci(interior, "finding:") {
        return clean_finding(rest).map(Reference::Finding);
    }
    // tsk<digits> — falls through to slug/file when the tail isn't
    // all-digits (e.g. `[[tskfoo]]` is a wiki slug, matching `extract`).
    if let Some(rest) = strip_prefix_ci(interior, "tsk") {
        if let Some(id) = parse_task_id(rest) {
            return Some(Reference::Task(id));
        }
    }
    // path/file.ext[@version][:line]
    if let Some(detail) = parse_file_ref(interior) {
        return Some(Reference::File(detail));
    }
    // bare commit sha or wiki slug
    let bare = interior.split(':').next().unwrap_or(interior);
    if !bare.is_empty() && looks_like_commit_sha(bare) {
        return Some(Reference::Commit(bare.to_string()));
    }
    if looks_like_slug(bare) {
        return Some(Reference::Wiki(bare.to_string()));
    }
    None
}

/// Classify every `[[…]]` wikilink in `body`. Code spans / fenced
/// blocks are masked first (via [`mask_code_regions`]) so illustrative
/// links inside them are skipped, mirroring [`extract`]. Interiors that
/// match no known ref shape come back with `reference: None`.
pub fn classify_wikilinks(body: &str) -> Vec<ClassifiedWikilink> {
    if body.is_empty() {
        return Vec::new();
    }
    let masked = mask_code_regions(body);
    let mut out = Vec::new();
    for cap in find_wikilinks(masked.as_str()) {
        let interior = cap.split('|').next().unwrap_or(cap).trim();
        if interior.is_empty() {
            continue;
        }
        out.push(ClassifiedWikilink {
            raw: interior.to_string(),
            reference: classify_interior(interior),
        });
    }
    out
}

/// Parse `body` into [`ExtractedRefs`]. Pure; never errors.
pub fn extract(body: &str) -> ExtractedRefs {
    if body.is_empty() {
        return ExtractedRefs::default();
    }
    // Neutralize code spans / fenced blocks first so illustrative
    // `[[...]]` or path tokens inside them never become refs.
    let masked = mask_code_regions(body);
    let body = masked.as_str();
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut wikis: BTreeSet<String> = BTreeSet::new();
    let mut tasks: BTreeSet<i64> = BTreeSet::new();
    let mut findings: BTreeSet<String> = BTreeSet::new();
    let mut commits: BTreeSet<String> = BTreeSet::new();
    let mut files_detail: Vec<FileRefDetail> = Vec::new();
    let mut files_seen: BTreeSet<(String, String, Option<u32>)> = BTreeSet::new();

    for cap in find_wikilinks(body) {
        let interior = cap.split('|').next().unwrap_or(cap).trim();
        match classify_interior(interior) {
            Some(Reference::Dir(d)) => {
                dirs.insert(d);
            }
            Some(Reference::Commit(sha)) => {
                commits.insert(sha);
            }
            Some(Reference::Finding(id)) => {
                findings.insert(id);
            }
            Some(Reference::Task(id)) => {
                tasks.insert(id);
            }
            Some(Reference::File(detail)) => {
                if files.insert(detail.path.clone()) {
                    let key = (
                        detail.path.clone(),
                        version_key(&detail.version),
                        detail.line,
                    );
                    if files_seen.insert(key) {
                        files_detail.push(detail);
                    }
                }
            }
            Some(Reference::Wiki(slug)) => {
                wikis.insert(slug);
            }
            None => {}
        }
    }

    // Inline mentions outside [[…]]. Strip URLs first so
    // path-shaped URL tails don't masquerade as file refs.
    let stripped = strip_urls(body);
    for path in find_inline_paths(&stripped) {
        if files.insert(path.clone()) {
            let key = (path.clone(), "disk".to_string(), None);
            if files_seen.insert(key) {
                files_detail.push(FileRefDetail {
                    path,
                    version: RefVersion::Disk,
                    line: None,
                });
            }
        }
    }
    for id in find_inline_tasks(&stripped) {
        tasks.insert(id);
    }
    for f in find_inline_findings(&stripped) {
        findings.insert(f);
    }

    ExtractedRefs {
        files: files.into_iter().collect(),
        files_detail,
        dirs: dirs.into_iter().collect(),
        wikis: wikis.into_iter().collect(),
        tasks: tasks.into_iter().collect(),
        findings: findings.into_iter().collect(),
        commits: commits.into_iter().collect(),
    }
}

fn version_key(v: &RefVersion) -> String {
    match v {
        RefVersion::Disk => "disk".to_string(),
        RefVersion::Ref(r) => format!("ref:{r}"),
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    if s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn parse_file_ref(interior: &str) -> Option<FileRefDetail> {
    let trimmed = interior.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (path_and_line, version) = match trimmed.split_once('@') {
        Some((path_part, version_part)) => {
            let (v, line_part) = match version_part.split_once(':') {
                Some((v, l)) => (v, Some(l)),
                None => (version_part, None),
            };
            let v = v.trim();
            let parsed = if v.eq_ignore_ascii_case("disk") || v.eq_ignore_ascii_case("local") {
                RefVersion::Disk
            } else if !v.is_empty() {
                RefVersion::Ref(v.to_string())
            } else {
                RefVersion::Disk
            };
            let pl = match line_part {
                Some(l) => format!("{path_part}:{l}"),
                None => path_part.to_string(),
            };
            (pl, parsed)
        }
        None => (trimmed.to_string(), RefVersion::Disk),
    };
    let (bare, line) = match path_and_line.rsplit_once(':') {
        Some((p, l)) if !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()) => {
            (p.to_string(), l.parse::<u32>().ok())
        }
        _ => (path_and_line.clone(), None),
    };
    if !looks_like_file(&bare) {
        return None;
    }
    Some(FileRefDetail {
        path: bare,
        version,
        line,
    })
}

fn clean_dir(raw: &str) -> Option<String> {
    let bare = raw.trim().trim_end_matches('/');
    if bare.is_empty() || bare.starts_with('/') || bare.contains("//") || bare.contains('\n') {
        return None;
    }
    Some(bare.to_string())
}

fn clean_commit(raw: &str) -> Option<String> {
    let bare = raw.trim();
    if looks_like_commit_sha(bare) {
        Some(bare.to_string())
    } else {
        None
    }
}

fn clean_finding(raw: &str) -> Option<String> {
    let bare = raw.trim();
    if bare.is_empty() || bare.contains(' ') || bare.contains('\n') {
        return None;
    }
    Some(bare.to_string())
}

fn looks_like_file(s: &str) -> bool {
    if !s.contains('/') {
        return false;
    }
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
    if matches!(s.len(), 7..=40) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn looks_like_commit_sha(s: &str) -> bool {
    matches!(s.len(), 7..=40) && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_task_id(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse::<i64>().ok()
}

fn find_wikilinks(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
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

fn strip_urls(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 < bytes.len() && &bytes[i..i + 3] == b"://" {
            // Walk back to start of scheme word.
            let mut start = i;
            while start > 0 {
                let c = bytes[start - 1];
                if c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.' {
                    start -= 1;
                } else {
                    break;
                }
            }
            // Truncate already-pushed scheme chars.
            let pushed_scheme = i - start;
            for _ in 0..pushed_scheme {
                out.pop();
            }
            // Walk forward to whitespace or terminator.
            let mut end = i + 3;
            while end < bytes.len() {
                let c = bytes[end];
                if c.is_ascii_whitespace() || c == b')' || c == b']' || c == b'"' || c == b'<' {
                    break;
                }
                end += 1;
            }
            out.push(' ');
            i = end;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn find_inline_paths(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Treat a leading `~/` as the start of a candidate (home-
        // relative path). The renderer / IPC layer is responsible for
        // expanding the tilde when opening; the ref graph just stores
        // it verbatim so the link is round-trippable.
        let tilde_start = c == b'~'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'/'
            && !is_path_join_prev(bytes, i);
        let path_char =
            c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' || c == b'/';
        if !path_char && !tilde_start {
            i += 1;
            continue;
        }
        if !tilde_start && is_path_join_prev(bytes, i) {
            i += 1;
            continue;
        }
        let start = i;
        if tilde_start {
            // Consume the `~` then fall through to the standard
            // path-char loop for the rest of the run.
            i += 1;
        }
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' || c == b'/' {
                i += 1;
            } else {
                break;
            }
        }
        let candidate = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
        let trimmed = candidate.trim_end_matches('.');
        if looks_like_file(trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// True if `bytes[i-1]` is a char that would extend a path token —
/// alnum or `/`. Used to reject extraction in the middle of a URL
/// tail or a join. Returns false at BOF.
fn is_path_join_prev(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let p = bytes[i - 1];
    p.is_ascii_alphanumeric() || p == b'/'
}

fn find_inline_tasks(body: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let prefix = b"tsk";
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + prefix.len() < bytes.len() {
        if (i == 0 || !is_id_boundary_char(bytes[i - 1]))
            && bytes[i..i + prefix.len()].eq_ignore_ascii_case(prefix)
        {
            let start = i + prefix.len();
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            // The character after the digits must not extend an id-ish
            // token (e.g. `tsk42abc` is not a valid task ref).
            let next_extends = end < bytes.len() && is_id_boundary_char(bytes[end]);
            if end > start && !next_extends {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    if let Ok(n) = s.parse::<i64>() {
                        out.push(n);
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn find_inline_findings(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let prefix = b"finding:";
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + prefix.len() < bytes.len() {
        if (i == 0 || !is_id_boundary_char(bytes[i - 1]))
            && bytes[i..i + prefix.len()].eq_ignore_ascii_case(prefix)
        {
            let start = i + prefix.len();
            let mut end = start;
            while end < bytes.len() {
                let c = bytes[end];
                if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                    end += 1;
                } else {
                    break;
                }
            }
            if end > start {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    out.push(s.to_string());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_id_boundary_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// Replace the contents of fenced code blocks and inline code spans
/// with spaces so ref extraction never treats illustrative code — e.g.
/// `` `[[path]]` `` written in prose — as a real link or file path.
/// Backticks/fences are ASCII, so every replaced byte becomes one
/// single-byte space and the output length equals the input length.
pub fn mask_code_regions(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    // (fence byte, run length) while inside a fenced block.
    let mut in_fence: Option<(u8, usize)> = None;
    for line in body.split_inclusive('\n') {
        let fence = leading_fence(line);
        match in_fence {
            Some((fc, flen)) => {
                blank_line(&mut out, line);
                // A closing fence is a run of the same char, length ≥ opener.
                if let Some((c, len)) = fence {
                    if c == fc && len >= flen {
                        in_fence = None;
                    }
                }
            }
            None => match fence {
                Some((c, len)) => {
                    blank_line(&mut out, line);
                    in_fence = Some((c, len));
                }
                None => mask_inline_code(&mut out, line),
            },
        }
    }
    out
}

/// A leading code fence on a line: 3+ backticks or 3+ tildes after
/// optional leading whitespace. Returns `(fence_byte, run_len)`.
fn leading_fence(line: &str) -> Option<(u8, usize)> {
    let t = line.trim_start();
    let bytes = t.as_bytes();
    let c = *bytes.first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let run = bytes.iter().take_while(|&&b| b == c).count();
    (run >= 3).then_some((c, run))
}

/// Push `line` with every byte except line terminators replaced by a
/// space.
fn blank_line(out: &mut String, line: &str) {
    for &b in line.as_bytes() {
        out.push(if b == b'\n' || b == b'\r' {
            b as char
        } else {
            ' '
        });
    }
}

/// Mask inline code spans within a single (non-fenced) line. A span
/// opens on a run of N backticks and closes on the next run of exactly
/// N backticks (CommonMark). Spans (backticks + contents) become
/// spaces; text outside spans is copied verbatim. An unterminated run
/// is not a span — the rest of the line is copied as-is.
fn mask_inline_code(out: &mut String, line: &str) {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut copy_from = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let open = i;
        let mut n = 0;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
            n += 1;
        }
        // Scan for a closing run of exactly n backticks.
        let mut j = i;
        let mut end = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let mut m = 0;
                while j < bytes.len() && bytes[j] == b'`' {
                    j += 1;
                    m += 1;
                }
                if m == n {
                    end = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        match end {
            Some(end) => {
                out.push_str(&line[copy_from..open]);
                for _ in open..end {
                    out.push(' ');
                }
                copy_from = end;
                i = end;
            }
            None => break, // unterminated — copy the remainder verbatim
        }
    }
    out.push_str(&line[copy_from..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body() {
        assert_eq!(extract(""), ExtractedRefs::default());
    }

    #[test]
    fn wikilink_file_with_version_and_line() {
        let r = extract("see [[src/app.rs@HEAD:42]] for context");
        assert_eq!(r.files, vec!["src/app.rs"]);
        assert_eq!(r.files_detail.len(), 1);
        assert_eq!(r.files_detail[0].path, "src/app.rs");
        assert_eq!(r.files_detail[0].version, RefVersion::Ref("HEAD".into()));
        assert_eq!(r.files_detail[0].line, Some(42));
    }

    #[test]
    fn wikilink_dir_and_slug_and_task_and_finding_and_commit() {
        let body = "[[dir:src/components]] and [[architecture]] and [[tsk42]] and [[finding:fnd-1]] and [[git:abcdef0]]";
        let r = extract(body);
        assert_eq!(r.dirs, vec!["src/components"]);
        assert_eq!(r.wikis, vec!["architecture"]);
        assert_eq!(r.tasks, vec![42]);
        assert_eq!(r.findings, vec!["fnd-1"]);
        assert_eq!(r.commits, vec!["abcdef0"]);
    }

    #[test]
    fn bare_hex_in_wikilink_is_commit() {
        let r = extract("[[abc1234567]]");
        assert_eq!(r.commits, vec!["abc1234567"]);
        assert!(r.wikis.is_empty());
    }

    #[test]
    fn inline_path_picked_up() {
        let r = extract("touched src/lib.rs in this commit");
        assert_eq!(r.files, vec!["src/lib.rs"]);
    }

    #[test]
    fn url_is_not_a_file_ref() {
        let r = extract("see https://example.com/foo.json");
        assert!(r.files.is_empty(), "got {:?}", r.files);
    }

    #[test]
    fn inline_task_and_finding_mention() {
        let r = extract("blocked by tsk42 see finding:fnd-2");
        assert_eq!(r.tasks, vec![42]);
        assert_eq!(r.findings, vec!["fnd-2"]);
    }

    #[test]
    fn inline_task_rejects_non_digits() {
        let r = extract("tskfoo and tsk42abc");
        assert!(r.tasks.is_empty());
    }

    #[test]
    fn inline_task_with_trailing_punctuation() {
        let r = extract("tsk42 fixes issue. also tsk7, see");
        assert_eq!(r.tasks, vec![7, 42]);
    }

    #[test]
    fn wikilink_task_rejects_non_digits() {
        let r = extract("[[tsknotanumber]] [[tsk42abc]]");
        assert!(r.tasks.is_empty());
    }

    #[test]
    fn dedup_across_wikilink_and_inline() {
        let r = extract("see [[src/lib.rs]] and src/lib.rs again");
        assert_eq!(r.files, vec!["src/lib.rs"]);
        assert_eq!(r.files_detail.len(), 1);
    }

    #[test]
    fn pipe_alias_stripped() {
        let r = extract("[[src/app.rs|application entry]]");
        assert_eq!(r.files, vec!["src/app.rs"]);
    }

    #[test]
    fn wikilink_task_form_does_not_become_slug() {
        let r = extract("[[tsk7]]");
        assert!(r.wikis.is_empty());
        assert_eq!(r.tasks, vec![7]);
    }

    #[test]
    fn classify_recognizes_each_kind() {
        let links = classify_wikilinks(
            "[[tsk7]] [[architecture]] [[src/lib.rs]] [[dir:src]] \
             [[git:abc1234]] [[finding:fnd-1]]",
        );
        let refs: Vec<Option<Reference>> = links.into_iter().map(|l| l.reference).collect();
        assert_eq!(refs[0], Some(Reference::Task(7)));
        assert_eq!(refs[1], Some(Reference::Wiki("architecture".into())));
        assert!(matches!(refs[2], Some(Reference::File(_))));
        assert_eq!(refs[3], Some(Reference::Dir("src".into())));
        assert_eq!(refs[4], Some(Reference::Commit("abc1234".into())));
        assert_eq!(refs[5], Some(Reference::Finding("fnd-1".into())));
    }

    #[test]
    fn classify_flags_github_style_ref_as_unrecognized() {
        // `#13` is the exact bug: a `[[…]]` interior that matches no known
        // ref shape → `reference: None`, so the link checker flags it.
        let links = classify_wikilinks("Follow-ups: [[#13]] and [[#14]].");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].raw, "#13");
        assert!(links[0].reference.is_none());
        assert!(links[1].reference.is_none());
    }

    #[test]
    fn classify_strips_label_and_ignores_code() {
        let links = classify_wikilinks("[[tsk7|the task]] but not `[[tsk8]]`.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].raw, "tsk7");
        assert_eq!(links[0].reference, Some(Reference::Task(7)));
    }

    #[test]
    fn wikilink_task_zero_extracts() {
        // `0` is a valid integer; the renderer is free to treat it as a
        // dead link (since SQLite never assigns 0), but the extractor
        // itself does not gatekeep — that's the renderer's job.
        let r = extract("[[tsk0]] inline tsk0");
        assert_eq!(r.tasks, vec![0]);
    }

    #[test]
    fn wikilink_task_negative_rejected() {
        // The wikilink grammar accepts only ASCII digits — the leading
        // `-` makes the body fail the digit check.
        let r = extract("[[tsk-1]] inline tsk-1");
        assert!(r.tasks.is_empty());
    }

    #[test]
    fn wikilink_task_overflow_rejected() {
        // i64 overflows are dropped silently (parse::<i64>() returns
        // None). We don't surface a parse error; the renderer just
        // doesn't see the ref.
        let r = extract("[[tsk99999999999999999999]]");
        assert!(r.tasks.is_empty());
    }

    #[test]
    fn inline_task_overflow_rejected() {
        let r = extract("see tsk99999999999999999999 in passing");
        assert!(r.tasks.is_empty());
    }

    #[test]
    fn inline_tilde_path_captured_with_leading_tilde() {
        let r = extract("see ~/.claude/plans/yes-plan-a-good-harmonic-floyd.md for details");
        assert_eq!(
            r.files,
            vec!["~/.claude/plans/yes-plan-a-good-harmonic-floyd.md"]
        );
    }

    #[test]
    fn inline_tilde_path_in_parens() {
        let r = extract("docs (~/notes/things.md) cover that");
        assert_eq!(r.files, vec!["~/notes/things.md"]);
    }

    #[test]
    fn bare_tilde_without_slash_is_not_a_path() {
        let r = extract("approximately ~5 items");
        assert!(r.files.is_empty(), "got {:?}", r.files);
    }

    #[test]
    fn wikilink_inside_inline_code_is_ignored() {
        // Regression: `` `[[path]]` `` in prose is illustrative, not a
        // link. Neither the wiki slug nor a file path should extract.
        let r = extract("normalized to the `[[some-slug]]` form, see `[[src/foo.rs]]`");
        assert!(r.wikis.is_empty(), "got wikis {:?}", r.wikis);
        assert!(r.files.is_empty(), "got files {:?}", r.files);
    }

    #[test]
    fn wikilink_inside_fenced_block_is_ignored() {
        let body = "before\n```\n[[src/foo.rs]] and [[in-fence-slug]]\n```\nafter [[real-slug]]";
        let r = extract(body);
        assert!(r.files.is_empty(), "got files {:?}", r.files);
        assert_eq!(r.wikis, vec!["real-slug"]);
    }

    #[test]
    fn inline_path_inside_code_span_is_ignored() {
        let r = extract("run `cargo test src/lib.rs` to check");
        assert!(r.files.is_empty(), "got {:?}", r.files);
    }

    #[test]
    fn link_outside_code_still_extracted_alongside_code() {
        let r = extract("see [[src/foo.rs]] and run `cargo build`");
        assert_eq!(r.files, vec!["src/foo.rs"]);
    }

    #[test]
    fn unterminated_backtick_does_not_swallow_rest() {
        // A lone backtick is not a code span — refs after it still parse.
        let r = extract("a stray ` then [[real-slug]]");
        assert_eq!(r.wikis, vec!["real-slug"]);
    }

    #[test]
    fn mask_preserves_byte_length() {
        let body = "x `code` y\n```\nfenced\n```\nz";
        assert_eq!(mask_code_regions(body).len(), body.len());
    }

    #[test]
    fn tilde_after_alnum_is_not_a_path_start() {
        // `foo~/bar.md` shouldn't trigger — the tilde isn't at a word
        // boundary so it's not a home-relative path.
        let r = extract("foo~/bar.md");
        assert!(
            !r.files.iter().any(|p| p.starts_with('~')),
            "got {:?}",
            r.files,
        );
    }
}
