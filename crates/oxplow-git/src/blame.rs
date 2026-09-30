//! `git blame` parsing.
//!
//! Shells out to `git blame --porcelain` and parses the output.
//! Porcelain format: each block opens with `<sha> <orig-line> <final-line> [count]`,
//! followed by zero-or-more `key value` header lines (only emitted on
//! first occurrence per sha), then a content line prefixed with `\t`.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;

pub const BLAME_ZERO_SHA: &str = "0000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlameLine {
    pub line: u32,
    pub sha: String,
    pub author: String,
    pub author_mail: String,
    pub author_time: i64,
    pub summary: String,
}

/// Who last changed each line of `path` as of `rev`, or of the working
/// file when `rev` is `None` (an uncommitted line then carries
/// [`BLAME_ZERO_SHA`]). An error carries git's own message (an unknown
/// revision, a path not in it).
pub fn git_blame(repo: &Path, rev: Option<&str>, path: &str) -> Result<Vec<BlameLine>, String> {
    let mut args = vec!["blame", "--porcelain"];
    args.extend(rev);
    args.extend(["--", path]);
    let output = Command::new("git")
        .args(&args)
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_porcelain(&String::from_utf8_lossy(&output.stdout)))
}

#[derive(Default, Clone)]
struct CommitMeta {
    author: String,
    author_mail: String,
    author_time: i64,
    summary: String,
}

pub fn parse_porcelain(raw: &str) -> Vec<BlameLine> {
    let lines: Vec<&str> = raw.split('\n').collect();
    let mut meta: HashMap<String, CommitMeta> = HashMap::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let header = lines[i];
        i += 1;
        if header.is_empty() {
            continue;
        }
        // Header form: `<sha> <orig-line> <final-line> [count]`
        let mut parts = header.split_whitespace();
        let sha = match parts.next() {
            Some(s) if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) => s.to_string(),
            _ => continue,
        };
        let _orig = parts.next();
        let final_line: u32 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        // Read header k/v lines until we hit the content line (starts with TAB).
        let mut fresh = meta.get(&sha).cloned().unwrap_or_default();
        let had_meta = meta.contains_key(&sha);
        while i < lines.len() && !lines[i].starts_with('\t') {
            let kv = lines[i];
            i += 1;
            if let Some(rest) = kv.strip_prefix("author ") {
                fresh.author = rest.to_string();
            } else if let Some(rest) = kv.strip_prefix("author-mail ") {
                fresh.author_mail = rest.trim_matches(|c| c == '<' || c == '>').to_string();
            } else if let Some(rest) = kv.strip_prefix("author-time ") {
                fresh.author_time = rest.parse().unwrap_or(0);
            } else if let Some(rest) = kv.strip_prefix("summary ") {
                fresh.summary = rest.to_string();
            }
        }
        // Skip the content line.
        if i < lines.len() && lines[i].starts_with('\t') {
            i += 1;
        }
        if !had_meta {
            meta.insert(sha.clone(), fresh.clone());
        }
        let entry = meta.get(&sha).cloned().unwrap_or(fresh);
        out.push(BlameLine {
            line: final_line,
            sha,
            author: entry.author,
            author_mail: entry.author_mail,
            author_time: entry.author_time,
            summary: entry.summary,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    #[test]
    fn parse_porcelain_extracts_per_line_metadata() {
        let raw = "abcdef0123456789abcdef0123456789abcdef01 1 1 1\n\
                   author Alice\n\
                   author-mail <a@example.com>\n\
                   author-time 1700000000\n\
                   summary first\n\
                   filename a.txt\n\
                   \thello\n\
                   abcdef0123456789abcdef0123456789abcdef01 2 2\n\
                   \tworld\n";
        let out = parse_porcelain(raw);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].author, "Alice");
        assert_eq!(out[0].author_mail, "a@example.com");
        assert_eq!(out[1].author, "Alice", "second hunk reuses cached metadata");
    }

    #[test]
    fn git_blame_against_real_repo() {
        let dir = tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["config", "user.email", "t@e.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["config", "user.name", "Tester"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nbeta\n").unwrap();
        Cmd::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let lines = git_blame(dir.path(), Some("HEAD"), "a.txt").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].author, "Tester");
    }
}
