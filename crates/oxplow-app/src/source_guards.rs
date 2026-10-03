//! Source-scan guards over the workspace's production code (P7.B6,
//! P8.A1, `.context/ipc-and-stores.md`): what the compiler can't hold —
//! who may push which UI event, and that the thin callers (the RPC, MCP
//! and control-plane layers) never write the database themselves.

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

/// The layers that only call into oxplow-app: the RPC dispatch, the MCP
/// tools and the control plane (hooks, OTLP). A write from one of them is
/// a command on the bus, or a service's own ingest — never its own.
fn thin_caller(path: &str) -> bool {
    [
        "crates/oxplow-rpc/",
        "crates/oxplow-mcp/",
        "crates/oxplow-control-plane/",
    ]
    .iter()
    .any(|p| path.starts_with(p))
}

/// P8.A1: the thin callers never open a transaction or call a store's
/// `_tx` core of their own.
#[test]
fn thin_callers_never_write_the_database_themselves() {
    let offenders: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, _)| thin_caller(path))
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

/// The writes the thin callers make through a store themselves, each with
/// the off-bus reason the doc's table gives (`ipc-and-stores.md` "What
/// stays off the bus") — the needle that table must contain.
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
    (
        "crates/oxplow-rpc/src/commands/page_visit.rs",
        "page_visit_store.record",
        "page visits",
    ),
    (
        "crates/oxplow-rpc/src/commands/page_visit.rs",
        "page_visit_store.forget_page",
        "forgetting a page",
    ),
];

/// A store method that only reads, by its name.
fn reads(method: &str) -> bool {
    const READS: [&str; 11] = [
        "get", "list", "read", "primary", "current", "selected", "stats", "find", "count",
        "search", "recent",
    ];
    READS
        .iter()
        .any(|r| method == *r || method.starts_with(&format!("{r}_")))
}

/// `text`'s code with its `//` comments gone and the whitespace around
/// each `.` removed, so a call chain split across lines
/// (`svc\n    .comment_store\n    .set_anchor(`) reads as one.
fn joined_code(text: &str) -> String {
    let code: String = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
            }
            // Before a `.` or after one, it is the chain's own layout.
            if chars.peek() == Some(&'.') || out.ends_with('.') {
                continue;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// The store writes in `text`: each `<x>_store.<method>(` call whose
/// method doesn't read, however the chain is laid out.
fn store_writes(text: &str) -> Vec<String> {
    let code = joined_code(text);
    let mut out = Vec::new();
    for (i, _) in code.match_indices("_store.") {
        let start = code[..i]
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |p| p + 1);
        let rest = &code[i + "_store.".len()..];
        let method: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if rest[method.len()..].starts_with('(') && !reads(&method) {
            out.push(format!("{}_store.{method}", &code[start..i]));
        }
    }
    out
}

/// tsk861: a call chain laid out over several lines is one call.
#[test]
fn a_multiline_store_write_is_caught() {
    let text = "Ok(svc\n        .comment_store // where it is now\n        .set_anchor(id, &s, o)\n        .await?)\nsvc.thread_store.get(id)";
    assert_eq!(store_writes(text), vec!["comment_store.set_anchor"]);
}

/// tsk785: a store call in a thin caller is a read, or a write listed in
/// `OFF_BUS` with its reason — and the doc's off-bus table names each
/// listed one. A write through a store's async method used to slip past
/// the transaction scan above, and one laid out over several lines past
/// this one (tsk861).
#[test]
fn thin_caller_store_writes_are_listed_off_the_bus() {
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let mut offenders = Vec::new();
    for (path, text) in production_sources() {
        if !thin_caller(&path) {
            continue;
        }
        for call in store_writes(&text) {
            if OFF_BUS.iter().any(|(p, c, _)| *p == path && *c == call) {
                found.insert((path.clone(), call));
            } else {
                offenders.push(format!("{path}: {call}"));
            }
        }
    }
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "a write through a store from a thin caller: make it a command, or list it in OFF_BUS and the doc's table"
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
    assert!(!has("crates/oxplow-app/src/test_fixtures.rs"));
}

/// What only a shim, alias or migrator for an old shape says (tsk865: no
/// legacy code, no migration code). The SQL schema migrations aren't
/// sources here — they upgrade the one database — and the two readers of
/// rows already in it (`SnapshotTrigger::Legacy`, the comment selectors'
/// position objects) don't say any of these.
const LEGACY_SHIMS: &[&str] = &[
    "migrate_v1",
    "migrate_gauges",
    "gauges_to_collectors",
    "GAUGES_RETIRED",
    "plugin migrate",
    "migrate_legacy",
    "hook_bridge.py",
    "\"legacy:",
    "#[deprecated",
    "@deprecated",
    "Legacy alias",
    "API compatibility",
    "for back-compat",
];

/// The desktop's own TypeScript, tests and generated bindings aside.
fn desktop_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack = vec![root.join("apps/desktop/src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "generated" {
                    stack.push(path);
                }
            } else if (name.ends_with(".ts") || name.ends_with(".tsx")) && !name.contains(".test.")
            {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    out
}

/// tsk865: no production source keeps a shim, alias or migrator for an
/// old shape — a manifest without `manifest: 2`, a `gauges:` block, a
/// `collection:` block, an old key are errors, not conversions.
#[test]
fn no_legacy_shims() {
    let found: Vec<String> = production_sources()
        .into_iter()
        .chain(desktop_sources())
        .flat_map(|(path, text)| {
            LEGACY_SHIMS
                .iter()
                .filter(|needle| text.contains(*needle))
                .map(|needle| format!("{path}: {needle}"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
}

/// The `[section]` each line of a Cargo manifest is in, paired with the
/// line.
fn manifest_lines(text: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                section = trimmed.to_string();
                None
            } else {
                Some((section.clone(), trimmed.to_string()))
            }
        })
        .collect()
}

/// The sections a development-only crate may be named in: a crate's
/// dev-dependencies, and the workspace's table of paths.
fn names_it_for_production(manifest: &str, krate: &str) -> Vec<String> {
    manifest_lines(manifest)
        .into_iter()
        .filter(|(_, line)| {
            line.starts_with(&format!("{krate} "))
                || line.starts_with(&format!("{krate}="))
                || line.starts_with(&format!("{krate}."))
        })
        .filter(|(section, _)| {
            !matches!(
                section.as_str(),
                "[dev-dependencies]" | "[workspace.dependencies]"
            ) && !section.ends_with(".dev-dependencies]")
        })
        .map(|(section, line)| format!("{section} {line}"))
        .collect()
}

/// P10: the stand-in OAuth server signs anyone in; it is the sign-in
/// tests' dev-dependency and a binary run by hand, never part of anything
/// that ships.
#[test]
fn the_oauth_sim_is_never_a_production_dependency() {
    assert_eq!(
        names_it_for_production(
            "[package]\nname = \"a\"\n[dependencies]\noxplow-oauth-sim = { workspace = true }\n[dev-dependencies]\noxplow-oauth-sim = { workspace = true }\n[target.'cfg(unix)'.dev-dependencies]\noxplow-oauth-sim.workspace = true\n",
            "oxplow-oauth-sim"
        ),
        vec!["[dependencies] oxplow-oauth-sim = { workspace = true }"]
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut manifests = vec![
        root.join("Cargo.toml"),
        root.join("apps/desktop/src-tauri/Cargo.toml"),
    ];
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        if manifest.exists() {
            manifests.push(manifest);
        }
    }
    let offenders: Vec<String> = manifests
        .iter()
        .filter(|m| !m.ends_with("crates/oxplow-oauth-sim/Cargo.toml"))
        .flat_map(|m| {
            let text = std::fs::read_to_string(m).unwrap();
            names_it_for_production(&text, "oxplow-oauth-sim")
                .into_iter()
                .map(move |l| format!("{}: {l}", m.display()))
        })
        .collect();
    assert!(offenders.is_empty(), "{offenders:#?}");
}

/// P10: a sign-in's redirect is caught by the desktop shell, never the
/// core — so it lands where the person's browser is, with a remote daemon
/// too. In the core's crates only `oauth_redirect.rs` (the shell's
/// listener) knows how to listen for one and nothing uses it; the
/// providers bind no socket at all.
#[test]
fn the_core_never_binds_a_socket_for_a_sign_in() {
    const CORE: &[&str] = &[
        "crates/oxplow-app/",
        "crates/oxplow-rpc/",
        "crates/oxplow-daemon/",
        "crates/oxplow-mcp/",
        "crates/oxplow-control-plane/",
    ];
    let offenders: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, _)| {
            CORE.iter().any(|c| path.starts_with(c))
                && path != "crates/oxplow-app/src/oauth_redirect.rs"
        })
        .filter(|(path, text)| {
            text.contains("RedirectListener")
                || (path.starts_with("crates/oxplow-app/src/providers/")
                    && text.contains("TcpListener"))
        })
        .map(|(path, _)| path)
        .collect();
    assert!(offenders.is_empty(), "{offenders:#?}");
}
