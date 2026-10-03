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
    let mut files: Vec<(PathBuf, String)> = Vec::new();
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
                files.push((path, text));
            }
        }
    }
    // A module declared `#[cfg(test)] mod x;` is test code wherever its
    // file is: `x.rs` and everything under `x/`.
    let test_only: Vec<PathBuf> = files
        .iter()
        .flat_map(|(path, text)| {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let parent = path.parent().unwrap_or(Path::new("")).to_path_buf();
            let dir = if matches!(stem.as_ref(), "mod" | "lib" | "main") {
                parent
            } else {
                parent.join(stem.as_ref())
            };
            test_only_modules(text)
                .into_iter()
                .map(move |name| dir.join(name))
        })
        .collect();
    let mut out: Vec<(String, String)> = files
        .iter()
        .filter(|(path, _)| {
            !test_only
                .iter()
                .any(|m| *path == m.with_extension("rs") || path.starts_with(m))
        })
        .map(|(path, text)| {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            (rel, production_part(text).to_string())
        })
        .collect();
    out.sort();
    out
}

/// The modules `text` declares for tests only: each `#[cfg(test)]`
/// followed by `mod <name>;` (whatever its visibility).
fn test_only_modules(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find("#[cfg(test)]") {
        at += i + "#[cfg(test)]".len();
        let rest = text[at..].trim_start();
        let rest = match rest.strip_prefix("pub") {
            Some(r) => match r.strip_prefix('(') {
                Some(scoped) => scoped.split_once(')').map_or(r, |(_, after)| after),
                None => r,
            },
            None => rest,
        }
        .trim_start();
        let Some(decl) = rest.strip_prefix("mod ") else {
            continue;
        };
        let name: String = decl
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if decl[name.len()..].trim_start().starts_with(';') {
            out.push(name);
        }
    }
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
#[rustfmt::skip]
const EMITTERS: &[(&str, &str)] = &[
    ("AgentStallAlert", "crates/oxplow-app/src/agent_stall_watch.rs"),
    ("AgentStatusChanged", "crates/oxplow-app/src/agent_stall_watch.rs"),
    ("AgentStatusChanged", "crates/oxplow-app/src/hook_ingest.rs"),
    ("BackgroundTasksChanged", "crates/oxplow-app/src/lib.rs"),
    ("FollowupsChanged", "crates/oxplow-app/src/lib.rs"),
    ("ConfigChanged", "crates/oxplow-app/src/commands/config_commands.rs"),
    ("ConfigChanged", "crates/oxplow-app/src/lib.rs"),
    ("ConfigChanged", "crates/oxplow-rpc/src/commands/ai.rs"),
    ("CredentialChanged", "crates/oxplow-app/src/providers/registry.rs"),
    ("LspServersChanged", "crates/oxplow-app/src/commands/lsp.rs"),
    ("MetricSamplesChanged", "crates/oxplow-app/src/models_changed.rs"),
    ("ModelsChanged", "crates/oxplow-app/src/models_changed.rs"),
    ("SnapshotTaken", "crates/oxplow-app/src/ui_push.rs"),
    ("StreamOrphaned", "crates/oxplow-app/src/workspace_watch.rs"),
    ("VcsRefsChanged", "crates/oxplow-app/src/ref_moves.rs"),
    ("WorkspaceChanged", "crates/oxplow-app/src/commands/vcs.rs"),
    ("WorkspaceChanged", "crates/oxplow-app/src/workspace_files.rs"),
    ("WorkspaceChanged", "crates/oxplow-app/src/workspace_watch.rs"),
    ("WorkspaceContextChanged", "crates/oxplow-app/src/workspace_watch.rs"),
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

/// The writes oxplow-rpc and oxplow-mcp make through a store themselves,
/// each with the off-bus reason the doc's table gives (`ipc-and-stores.md`
/// "What stays off the bus") — the needle that table must contain.
const OFF_BUS: &[(&str, &str, &str)] = &[
    (
        "crates/oxplow-rpc/src/commands/semantic.rs",
        "panel_layout_store.set",
        "left-nav panel layout",
    ),
    (
        "crates/oxplow-rpc/src/commands/usage.rs",
        "usage_store.record",
        "usage recording",
    ),
];

/// A store method that only reads, by its name.
fn reads(method: &str) -> bool {
    const READS: [&str; 10] = [
        "get", "list", "read", "primary", "current", "selected", "stats", "find", "count", "search",
    ];
    READS
        .iter()
        .any(|r| method == *r || method.starts_with(&format!("{r}_")))
}

/// tsk785: a store call in oxplow-rpc / oxplow-mcp is a read, or a write
/// listed in `OFF_BUS` with its reason — and the doc's off-bus table names
/// each listed one. A write through a store's async method used to slip
/// past the transaction scan above.
#[test]
fn rpc_and_mcp_store_writes_are_listed_off_the_bus() {
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let mut offenders = Vec::new();
    for (path, text) in production_sources() {
        if !(path.starts_with("crates/oxplow-rpc/") || path.starts_with("crates/oxplow-mcp/")) {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for (i, _) in code.match_indices("_store.") {
                let start = code[..i]
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .map_or(0, |p| p + 1);
                let rest = &code[i + "_store.".len()..];
                let method: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !rest[method.len()..].starts_with('(') || reads(&method) {
                    continue;
                }
                let call = format!("{}_store.{method}", &code[start..i]);
                if OFF_BUS.iter().any(|(p, c, _)| *p == path && *c == call) {
                    found.insert((path.clone(), call));
                } else {
                    offenders.push(format!("{path}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "a write through a store from RPC/MCP: make it a command, or list it in OFF_BUS and the doc's table"
    );
    let stale: Vec<_> = OFF_BUS
        .iter()
        .filter(|(p, c, _)| !found.contains(&(p.to_string(), c.to_string())))
        .collect();
    assert!(stale.is_empty(), "OFF_BUS rows no longer called: {stale:?}");
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.context/ipc-and-stores.md"),
    )
    .unwrap();
    let missing: Vec<_> = OFF_BUS
        .iter()
        .filter(|(_, _, needle)| !doc.contains(needle))
        .collect();
    assert!(
        missing.is_empty(),
        "OFF_BUS rows the doc's table doesn't give: {missing:?}"
    );
}

/// The `OxplowEvent` variants, as the wire names them (`kind`, camelCase).
fn event_kinds() -> Vec<String> {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/events.rs"))
        .unwrap();
    let body = &src[src.find("pub enum OxplowEvent {").unwrap()..];
    let body = &body[..body.find("\n}").unwrap()];
    body.lines()
        .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
        .map(str::trim)
        .filter(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
        .map(|l| {
            let name: String = l.chars().take_while(|c| c.is_alphanumeric()).collect();
            let mut kind = name[..1].to_lowercase();
            kind.push_str(&name[1..]);
            kind
        })
        .collect()
}

/// P8.A2: every UI event has a listener in the renderer — one nobody hears
/// is a write path's leftover, and its readers re-read a model instead.
#[test]
fn every_ui_event_has_a_listener() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src");
    let mut text = String::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "generated" {
                    stack.push(path);
                }
            } else if (name.ends_with(".ts") || name.ends_with(".tsx")) && !name.contains(".test.")
            {
                text.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    let unheard: Vec<String> = event_kinds()
        .into_iter()
        .filter(|k| !text.contains(&format!("\"{k}\"")))
        .collect();
    assert_eq!(unheard, Vec::<String>::new());
}

/// tsk789: every UI event has a pinned emitter — a variant the renderer
/// listens for that nothing sends leaves its view stale.
#[test]
fn every_ui_event_has_an_emitter() {
    let emitted: BTreeSet<String> = EMITTERS
        .iter()
        .map(|(variant, _)| {
            let mut kind = variant[..1].to_lowercase();
            kind.push_str(&variant[1..]);
            kind
        })
        .collect();
    let silent: Vec<String> = event_kinds()
        .into_iter()
        .filter(|k| !emitted.contains(k))
        .collect();
    assert_eq!(silent, Vec::<String>::new());
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

/// The scan's own rule: a module declared for tests only isn't production,
/// whatever its file is called (`providers/tests.rs`, `test_fixtures.rs`).
#[test]
fn a_test_only_module_is_not_production() {
    assert_eq!(
        test_only_modules(
            "pub mod a;\n#[cfg(test)]\npub mod sim;\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\npub(crate) mod fixtures;\n#[cfg(test)]\nmod inline { }\n#[cfg(test)]\nfn helper() {}\n"
        ),
        vec!["sim", "tests", "fixtures"]
    );
    let sources: Vec<String> = production_sources().into_iter().map(|(p, _)| p).collect();
    let has = |p: &str| sources.iter().any(|s| s == p);
    assert!(has("crates/oxplow-app/src/providers/registry.rs"));
    assert!(!has("crates/oxplow-app/src/providers/tests.rs"));
    assert!(!has("crates/oxplow-app/src/providers/oauth_sim.rs"));
    assert!(!has("crates/oxplow-app/src/test_fixtures.rs"));
}
