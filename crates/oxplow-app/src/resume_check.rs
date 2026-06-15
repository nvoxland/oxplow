//! Launch-time validation of a thread's Claude resume pointer.
//!
//! `thread.resume_session_id` drives `claude --resume <id>`. When the
//! id points at a session whose transcript is gone (machine moved, id
//! rotted, history pruned), Claude prints a raw `No conversation found
//! with session ID …` error before our shell `||` net falls back to a
//! fresh session — and the stale id lingers in the DB until the next
//! `UserPromptSubmit` self-heals it (Claude Code drops HTTP hooks for
//! `SessionStart`, so nothing fires sooner — see
//! `.context/agent-model.md`). Checking the session file at launch lets
//! the caller blank the pointer and start fresh *without* triggering
//! the raw error.

use std::path::Path;

/// Outcome of probing whether a Claude session transcript still exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeState {
    /// The session `.jsonl` is on disk — `--resume` will work.
    Present,
    /// The project dir exists but the session `.jsonl` is gone — the id
    /// is stale and should be cleared.
    Missing,
    /// We can't tell (no `~/.claude/projects/<cwd>` dir — fresh machine,
    /// never-launched cwd, or an encoding mismatch). Stay conservative:
    /// the caller leaves the pointer alone and the shell `||` net still
    /// protects the launch.
    Unknown,
}

/// Claude encodes a session's cwd into its projects-dir name by
/// replacing every non-alphanumeric byte with `-`, e.g.
/// `/Users/x/src/oxplow` -> `-Users-x-src-oxplow`. Confirmed against a
/// live `~/.claude/projects/` listing.
fn encode_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Probe whether the Claude session transcript for `session_id` (launched
/// in `cwd`) still exists under `<home>/.claude/projects/`.
///
/// `home` is passed explicitly (call site resolves `$HOME`, mirroring
/// `token_usage`) so this stays unit-testable against a temp dir.
pub fn claude_resume_state(home: &Path, cwd: &str, session_id: &str) -> ResumeState {
    if session_id.is_empty() {
        return ResumeState::Unknown;
    }
    let project_dir = home.join(".claude").join("projects").join(encode_cwd(cwd));
    if !project_dir.is_dir() {
        return ResumeState::Unknown;
    }
    if project_dir.join(format!("{session_id}.jsonl")).is_file() {
        ResumeState::Present
    } else {
        ResumeState::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn encodes_path_separators_and_dots() {
        assert_eq!(
            encode_cwd("/Users/nv/src/nvoxland/oxplow"),
            "-Users-nv-src-nvoxland-oxplow"
        );
        // Dots and underscores are non-alphanumeric -> dashes too.
        assert_eq!(encode_cwd("/a/b.c_d"), "-a-b-c-d");
    }

    fn project_dir(home: &Path, cwd: &str) -> std::path::PathBuf {
        home.join(".claude").join("projects").join(encode_cwd(cwd))
    }

    #[test]
    fn present_when_session_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/repo/wt";
        let dir = project_dir(tmp.path(), cwd);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sess-123.jsonl"), "{}").unwrap();
        assert_eq!(
            claude_resume_state(tmp.path(), cwd, "sess-123"),
            ResumeState::Present
        );
    }

    #[test]
    fn missing_when_dir_exists_but_file_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/repo/wt";
        fs::create_dir_all(project_dir(tmp.path(), cwd)).unwrap();
        assert_eq!(
            claude_resume_state(tmp.path(), cwd, "gone"),
            ResumeState::Missing
        );
    }

    #[test]
    fn unknown_when_project_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            claude_resume_state(tmp.path(), "/never/launched", "whatever"),
            ResumeState::Unknown
        );
    }

    #[test]
    fn unknown_when_session_id_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            claude_resume_state(tmp.path(), "/repo/wt", ""),
            ResumeState::Unknown
        );
    }
}
