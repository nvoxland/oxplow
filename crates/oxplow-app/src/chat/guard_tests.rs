//! Source-scan guard: oxplow never synthesizes agent input. An observed
//! Codex session's person prompts in Codex's own TUI; oxplow only injects
//! context between turns. This catches a path around that (a Codex
//! `turn/start`, a second injector). The TS side has
//! `no-agent-input-automation.test.ts`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

/// Every non-test Rust source line under `crates/*/src` and the Tauri
/// app, as (repo-relative path, line). Comment lines and `#[cfg(test)]`
/// test files are skipped.
fn source_lines() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path().join("src")))
        .collect();
    dirs.push(root.join("apps/desktop/src-tauri/src"));
    for dir in dirs {
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(p).unwrap();
            // Inline `mod tests` blocks come last by convention; stop there.
            for line in text.lines().take_while(|l| !l.starts_with("#[cfg(test)]")) {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                out.push((rel.clone(), line.to_string()));
            }
        }
    }
    out
}

/// `needle` in `line` not preceded by an identifier character, so
/// `compose(` doesn't match `decompose(`.
fn contains_word(line: &str, needle: &str) -> bool {
    line.match_indices(needle).any(|(i, _)| {
        !line[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

fn files_containing(lines: &[(String, String)], needle: &str) -> Vec<String> {
    let mut files: Vec<String> = lines
        .iter()
        .filter(|(_, l)| contains_word(l, needle))
        .map(|(f, _)| f.clone())
        .collect();
    files.dedup();
    files
}

/// Nothing in production starts a Codex turn: the person prompts in
/// Codex's own TUI, and oxplow only observes the thread.
#[test]
fn nothing_starts_a_codex_turn() {
    let lines = source_lines();
    assert!(files_containing(&lines, "\"turn/start\"").is_empty());
}

/// The one way context reaches a Codex thread mid-session is an item
/// appended to its history, from the Codex connection; it starts no turn,
/// and the session sends it only while none runs.
#[test]
fn only_the_codex_connection_injects_context() {
    let lines = source_lines();
    assert_eq!(
        files_containing(&lines, "\"thread/inject_items\""),
        vec!["crates/oxplow-app/src/chat/codex.rs"]
    );
}
