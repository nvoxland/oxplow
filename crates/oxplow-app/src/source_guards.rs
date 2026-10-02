//! Source-scan guards over the workspace's production code (P7.B6,
//! P8.A1, `.context/ipc-and-stores.md`): what the compiler can't hold —
//! who may push which UI event, and that the RPC and MCP layers never
//! write the database themselves.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every Rust source file in the workspace's crates and the desktop
/// shell, repo-relative, with its text before its test module.
pub fn production_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> =
        vec![root.join("crates"), root.join("apps/desktop/src-tauri/src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !matches!(name.as_str(), "tests" | "target" | "fixtures" | "examples") {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && name != "source_guards.rs" {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, production_part(&text).to_string()));
            }
        }
    }
    out.sort();
    out
}

/// `text` up to its test module (`#[cfg(test)]` before a `mod`).
fn production_part(text: &str) -> &str {
    let mut at = 0;
    while let Some(i) = text[at..].find("#[cfg(test)]") {
        let start = at + i;
        let rest = text[start + "#[cfg(test)]".len()..].trim_start();
        if rest.starts_with("mod tests") || rest.starts_with("pub(crate) mod tests") {
            return &text[..start];
        }
        at = start + 1;
    }
    text
}

/// Every `(variant, file)` that pushes `OxplowEvent::<variant>` onto the UI
/// bus (`….emit(OxplowEvent::…)`, whitespace ignored).
fn emitters() -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for (path, text) in production_sources() {
        let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let mut at = 0;
        while let Some(i) = flat[at..].find("emit(") {
            let start = at + i + "emit(".len();
            at = start;
            let call = &flat[start..];
            let Some(j) = call.find("OxplowEvent::") else {
                continue;
            };
            // Only a path to the variant (`OxplowEvent::X` or
            // `crate::events::OxplowEvent::X`), not something later.
            if !call[..j]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
            {
                continue;
            }
            let variant: String = call[j + "OxplowEvent::".len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect();
            out.insert((variant, path.clone()));
        }
    }
    out
}

/// Who may push each UI event. The renderer hears a model's change through
/// `ModelsChanged` and the log's facts through `ui.push`; what's left is a
/// UI-only signal with no durable fact behind it (its reason in
/// `.context/ipc-and-stores.md` → "Event bus"). Rows marked `P8.A` go as
/// their writes move onto the command bus and the desktop reads the model.
const EMITTERS: &[(&str, &str)] = &[
    ("AgentNudgesChanged", "crates/oxplow-app/src/collection.rs"), // P8.A2
    (
        "AgentStallAlert",
        "crates/oxplow-app/src/agent_stall_watch.rs",
    ),
    (
        "AgentStatusChanged",
        "crates/oxplow-app/src/agent_stall_watch.rs",
    ),
    ("AgentStatusChanged", "crates/oxplow-app/src/hook_ingest.rs"),
    (
        "AgentTokenUsageChanged",
        "crates/oxplow-app/src/token_usage.rs",
    ), // P8.A2
    ("AgentTurnsChanged", "crates/oxplow-app/src/hook_ingest.rs"), // P8.A10
    ("BackgroundTasksChanged", "crates/oxplow-app/src/lib.rs"),
    ("CommentsChanged", "crates/oxplow-mcp/src/lib.rs"), // P8.A6
    (
        "CommentsChanged",
        "crates/oxplow-rpc/src/commands/comments.rs",
    ), // P8.A6
    (
        "ConfigChanged",
        "crates/oxplow-app/src/commands/config_commands.rs",
    ),
    ("ConfigChanged", "crates/oxplow-app/src/lib.rs"),
    ("ConfigChanged", "crates/oxplow-rpc/src/commands/ai.rs"),
    ("CurrentStreamChanged", "crates/oxplow-mcp/src/lib.rs"),
    (
        "CurrentStreamChanged",
        "crates/oxplow-rpc/src/commands/streams.rs",
    ),
    ("DashboardsChanged", "crates/oxplow-mcp/src/lib.rs"), // P8.A5
    (
        "DashboardsChanged",
        "crates/oxplow-rpc/src/commands/dashboards.rs",
    ), // P8.A5
    (
        "DiagnosticsChanged",
        "crates/oxplow-app/src/lsp_diagnostics.rs",
    ), // P8.A2
    (
        "EffortObservationsChanged",
        "crates/oxplow-app/src/collection.rs",
    ), // P8.A2
    ("HookEventsChanged", "crates/oxplow-app/src/hook_ingest.rs"), // P8.A10
    ("HookEventsChanged", "crates/oxplow-app/src/recovery.rs"), // P8.A10
    ("LspServersChanged", "crates/oxplow-mcp/src/lib.rs"), // P8.A9
    ("LspServersChanged", "crates/oxplow-rpc/src/commands/lsp.rs"), // P8.A9
    (
        "MetricSamplesChanged",
        "crates/oxplow-app/src/models_changed.rs",
    ),
    ("ModelsChanged", "crates/oxplow-app/src/models_changed.rs"),
    (
        "PageVisitChanged",
        "crates/oxplow-rpc/src/commands/page_visit.rs",
    ), // P8.A10
    ("SelectedThreadChanged", "crates/oxplow-mcp/src/lib.rs"),
    (
        "SelectedThreadChanged",
        "crates/oxplow-rpc/src/commands/threads.rs",
    ),
    ("SnapshotTaken", "crates/oxplow-app/src/ui_push.rs"),
    ("StreamOrphaned", "crates/oxplow-app/src/workspace_watch.rs"),
    (
        "StreamsChanged",
        "crates/oxplow-app/src/branch_reconciler.rs",
    ), // P8.A4
    ("StreamsChanged", "crates/oxplow-app/src/workspace_watch.rs"), // P8.A4
    ("StreamsChanged", "crates/oxplow-mcp/src/lib.rs"),             // P8.A4
    (
        "StreamsChanged",
        "crates/oxplow-rpc/src/commands/streams.rs",
    ), // P8.A4
    ("ThreadsChanged", "crates/oxplow-mcp/src/lib.rs"),             // P8.A3
    (
        "ThreadsChanged",
        "crates/oxplow-rpc/src/commands/threads.rs",
    ), // P8.A3
    ("UsageRecorded", "crates/oxplow-rpc/src/commands/usage.rs"),   // P8.A10
    ("VcsRefsChanged", "crates/oxplow-app/src/ref_moves.rs"),
    ("WorkspaceChanged", "crates/oxplow-app/src/commands/vcs.rs"),
    (
        "WorkspaceChanged",
        "crates/oxplow-app/src/workspace_files.rs",
    ),
    (
        "WorkspaceChanged",
        "crates/oxplow-app/src/workspace_watch.rs",
    ),
    (
        "WorkspaceContextChanged",
        "crates/oxplow-app/src/workspace_watch.rs",
    ),
];

/// P8.A1: each UI event has the sources pinned in [`EMITTERS`] — a new
/// push is a decision (is there a model to re-read instead?), not a
/// convenience.
#[test]
fn ui_events_have_their_pinned_sources() {
    let pinned: BTreeSet<(String, String)> = EMITTERS
        .iter()
        .map(|(v, f)| (v.to_string(), f.to_string()))
        .collect();
    let found = emitters();
    let unlisted: Vec<_> = found.difference(&pinned).collect();
    let gone: Vec<_> = pinned.difference(&found).collect();
    assert!(
        unlisted.is_empty() && gone.is_empty(),
        "UI event sources changed.\nunlisted (add to EMITTERS with a reason, or read a model): {unlisted:#?}\nno longer emitted (drop from EMITTERS): {gone:#?}"
    );
}

/// P8.A1: the RPC and MCP layers are thin callers — a write is a command
/// on the bus, never a transaction or a store's `_tx` core of their own.
#[test]
fn rpc_and_mcp_never_write_the_database_themselves() {
    let offenders: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, _)| {
            path.starts_with("crates/oxplow-rpc/") || path.starts_with("crates/oxplow-mcp/")
        })
        .flat_map(|(path, text)| {
            text.lines()
                .enumerate()
                .filter(|(_, line)| {
                    let code = line.split("//").next().unwrap_or("");
                    code.contains(".transaction(")
                        || code.contains(".rehearse(")
                        || code
                            .match_indices("_tx(")
                            .any(|(i, _)| code[..i].ends_with(|c: char| c.is_alphanumeric()))
                })
                .map(|(n, line)| format!("{path}:{}: {}", n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(offenders, Vec::<String>::new());
}

/// P7.B6's guard: nothing listens to the in-memory bus but the `/events`
/// forwarder that hands it to the renderer. Backend work listens to the
/// event pump, an asset, the ref moves or the extension catalog's signal.
#[test]
fn the_bus_has_one_listener() {
    let allowed = [
        "crates/oxplow-daemon/src/lib.rs",
        "crates/oxplow-app/src/events.rs",
    ];
    let listeners: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, text)| text.contains(".subscribe_ui(") && !allowed.contains(&path.as_str()))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(listeners, Vec::<String>::new());
}
