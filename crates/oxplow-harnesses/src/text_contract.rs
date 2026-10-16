//! The text contract's launch half (`.context/agent-model.md` "What every
//! agent is told"): a launch of every built-in harness but ACP's (whose
//! agent is the project's program) delivers oxplow's system prompt, every
//! skill and every slash command to where its agent reads them — its
//! command line, its environment, the request that opens its thread, or a
//! file it was pointed at. Hook context is the other half: a harness declares the
//! channels its hooks deliver (`prompt_context`, `tool_context`) and core
//! sends nothing through one it lacks (`oxplow_app::agent_context`).

use std::collections::BTreeSet;
use std::path::Path;

use oxplow_domain::agent::harness::LaunchSpec;

use crate::test_launch::{harness, launch_at_home};

/// What a harness's launch doesn't deliver yet, each a gap to close (and
/// then drop from here): `(harness, what)`.
const KNOWN_GAPS: &[(&str, &str)] = &[];

const SYSTEM_PROMPT: &str = "OXPLOW-TEXT-CONTRACT-SYSTEM-PROMPT";

/// Every string a launch hands its agent: what its spec carries (a PTY's
/// command line and env values, an app-server thread's opening request),
/// each file it wrote under the project's runtime or the worktree — and,
/// for any of them that is JSON, every string inside it (opencode's inline
/// config carries its commands).
fn delivered(project: &Path, workspace: &Path, spec: &LaunchSpec) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                out.push(text);
            }
        }
    }
    fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
            serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
            _ => {}
        }
    }
    let mut out = match spec {
        LaunchSpec::Pty { command, env } => std::iter::once(command.clone())
            .chain(env.iter().map(|(_, v)| v.clone()))
            .collect(),
        LaunchSpec::CodexAppServer { params, .. } => vec![params.to_string()],
        LaunchSpec::Acp { .. } => panic!("an ACP agent's program is the project's"),
    };
    walk(&project.join(".oxplow/runtime"), &mut out);
    walk(workspace, &mut out);
    let json: Vec<String> = out
        .iter()
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .flat_map(|value| {
            let mut leaves = Vec::new();
            strings(&value, &mut leaves);
            leaves
        })
        .collect();
    out.extend(json);
    out
}

/// What of the contract a launch of `h` leaves out.
fn missing(entry: &str, id: &str) -> BTreeSet<(String, String)> {
    let h = harness(entry, id);
    let launched = launch_at_home(h.as_ref(), None, false, Some(SYSTEM_PROMPT));
    let haystack = delivered(
        &launched.project,
        &launched.workspace,
        &launched.launch.spec,
    );
    let reaches = |needle: &str| haystack.iter().any(|text| text.contains(needle));
    let text = oxplow_agent_text::core_text();
    let mut wanted = vec![("system prompt".to_string(), SYSTEM_PROMPT.to_string())];
    wanted.extend(
        text.skills
            .iter()
            .map(|s| (format!("skill {}", s.name), s.body.clone())),
    );
    // A command's prompt is its body; a harness may carry its frontmatter
    // as its own fields.
    wanted.extend(text.commands.iter().map(|c| {
        let prompt = c
            .body
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map_or(c.body.as_str(), |(_, body)| body);
        (format!("command {}", c.name), prompt.trim().to_string())
    }));
    wanted
        .into_iter()
        .filter(|(_, body)| !reaches(body))
        .map(|(what, _)| (id.to_string(), what))
        .collect()
}

#[test]
fn every_harness_delivers_the_prompt_skills_and_commands() {
    let mut gaps = BTreeSet::new();
    for (entry, id) in [
        ("oxplow:claude-code", "claude"),
        ("oxplow:codex-cli", "codex"),
        ("oxplow:opencode", "opencode"),
    ] {
        gaps.extend(missing(entry, id));
    }
    let known: BTreeSet<(String, String)> = KNOWN_GAPS
        .iter()
        .map(|(h, w)| (h.to_string(), w.to_string()))
        .collect();
    let new: Vec<_> = gaps.difference(&known).collect();
    let closed: Vec<_> = known.difference(&gaps).collect();
    assert!(
        new.is_empty() && closed.is_empty(),
        "what launches deliver changed.\nnewly missing: {new:#?}\nnow delivered (drop from KNOWN_GAPS): {closed:#?}"
    );
}
