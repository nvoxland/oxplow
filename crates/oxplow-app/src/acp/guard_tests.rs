//! Source-scan guard (tsk281): oxplow never synthesizes agent input. A
//! prompt reaches an ACP agent only because a person submitted it, so the
//! code that can build or send one is pinned here. `HumanPrompt` enforces
//! the same at the type level; this catches a new path around it (a raw
//! `session/prompt`, a second builder, a new caller of the submit path).
//! The TS side has `no-agent-input-automation.test.ts`.

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
/// `PromptRequest` doesn't match `SetThreadPromptRequest`.
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

const FAKE: &str = "crates/oxplow-acp-fake/";

#[test]
fn only_wire_names_the_sdk_prompt_request() {
    let lines = source_lines();
    assert_eq!(
        files_containing(&lines, "PromptRequest"),
        vec!["crates/oxplow-app/src/acp/wire.rs"]
    );
}

#[test]
fn no_code_sends_a_raw_session_prompt() {
    let lines = source_lines();
    let hits: Vec<String> = files_containing(&lines, "\"session/prompt\"")
        .into_iter()
        .filter(|f| !f.starts_with(FAKE))
        .collect();
    assert!(hits.is_empty(), "raw session/prompt in {hits:?}");
}

#[test]
fn prompts_are_composed_only_on_the_human_submit_path() {
    let lines = source_lines();
    let calls: Vec<&(String, String)> = lines
        .iter()
        .filter(|(f, l)| contains_word(l, "compose(") && !f.ends_with("acp/human_prompt.rs"))
        .collect();
    assert_eq!(calls.len(), 1, "compose callers: {calls:?}");
    assert_eq!(calls[0].0, "crates/oxplow-app/src/acp/session.rs");
    assert!(calls[0].1.contains("human_prompt::compose(&text"));
}

#[test]
fn only_the_prompt_box_command_submits_human_prompts() {
    let lines = source_lines();
    let allowed = [
        // The definition.
        "crates/oxplow-app/src/acp/manager.rs",
        // `acp_prompt`, the prompt box's Enter (a `ui(...)` row).
        "crates/oxplow-rpc/src/commands/acp.rs",
    ];
    let hits: Vec<String> = files_containing(&lines, "submit_human_prompt(")
        .into_iter()
        .filter(|f| !allowed.contains(&f.as_str()))
        .collect();
    assert!(
        hits.is_empty(),
        "unexpected submit_human_prompt callers: {hits:?}"
    );
}
