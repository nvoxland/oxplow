//! Source-scan guards over the workspace's production code (P7.B6,
//! P8.A1, `.context/ipc-and-stores.md`): who may push which UI event, and
//! the thin callers' generated clippy configs, which deny them every
//! database write by type (tsk903).

mod db_writes;

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
    // A person's approval is per machine, in no model (tsk1040).
    ("ApprovalsChanged", "crates/oxplow-rpc/src/commands/collectors.rs"),
    ("AgentStatusChanged", "crates/oxplow-app/src/agent_stall_watch.rs"),
    ("AgentStatusChanged", "crates/oxplow-app/src/hook_ingest.rs"),
    ("BackgroundTasksChanged", "crates/oxplow-app/src/lib.rs"),
    ("FollowupsChanged", "crates/oxplow-app/src/lib.rs"),
    ("ConfigChanged", "crates/oxplow-app/src/commands/config_commands.rs"),
    ("ConfigChanged", "crates/oxplow-app/src/lib.rs"),
    ("ConfigChanged", "crates/oxplow-rpc/src/commands/ai.rs"),
    ("CredentialChanged", "crates/oxplow-app/src/providers/registry.rs"),
    // The extension catalog's own signal (tsk1030): what extensions
    // contribute isn't a model to re-read, and a file path was a guess.
    ("ExtensionsChanged", "crates/oxplow-app/src/boot.rs"),
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

/// The crates that only call into oxplow-app: the RPC dispatch, the MCP
/// tools, the control plane (hooks, OTLP), the daemon and the desktop
/// shell. A write from one of them is a command on the bus, or a
/// service's own ingest — never its own.
const THIN_CALLERS: [&str; 6] = [
    "crates/oxplow-rpc",
    "crates/oxplow-mcp",
    "crates/oxplow-control-plane",
    "crates/oxplow-daemon",
    "crates/oxplow-tauri-ipc",
    "apps/desktop/src-tauri",
];

fn thin_caller(path: &str) -> bool {
    THIN_CALLERS
        .iter()
        .any(|p| path.starts_with(&format!("{p}/")))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A thin caller's `clippy.toml`: the root one's settings, and every
/// database write, consent and credential call denied by type
/// ([`db_writes::denied`]).
fn thin_caller_clippy_toml() -> String {
    let root = repo_root();
    let base = std::fs::read_to_string(root.join("clippy.toml")).unwrap();
    let mut out = String::from(
        "# Generated by crates/oxplow-app/src/source_guards.rs\n\
         # (`thin_caller_clippy_configs_are_current`); rewrite it with\n\
         # OXPLOW_BLESS=1. The root clippy.toml's settings, then every\n\
         # database write, consent and credential call this crate may not\n\
         # make (tsk903, .context/ipc-and-stores.md).\n\n",
    );
    out.push_str(&base);
    out.push_str("\ndisallowed-methods = [\n");
    for (path, reason) in db_writes::denied(&root) {
        out.push_str(&format!(
            "  {{ path = \"{path}\", reason = \"{reason}\" }},\n"
        ));
    }
    out.push_str("]\n");
    out
}

/// tsk903: each thin caller's clippy denies, by type, what it may not
/// call — an alias, a store built in place, a trait path or a turbofish
/// is the same call to the compiler. The list is generated; a change to a
/// store regenerates it.
#[test]
fn thin_caller_clippy_configs_are_current() {
    let want = thin_caller_clippy_toml();
    let bless = std::env::var("OXPLOW_BLESS").is_ok_and(|v| v == "1");
    let mut stale = Vec::new();
    for krate in THIN_CALLERS {
        let file = repo_root().join(krate).join("clippy.toml");
        if bless {
            std::fs::write(&file, &want).unwrap();
        } else if std::fs::read_to_string(&file).ok().as_deref() != Some(want.as_str()) {
            stale.push(krate);
        }
    }
    assert!(
        stale.is_empty(),
        "stale clippy.toml (run with OXPLOW_BLESS=1): {stale:?}"
    );
}

/// tsk903: a write is told by what its body does, not by its name — a
/// `get_or_create` writes, a trait's write is denied through the trait,
/// and a read through `Database::read` (always rolled back) is a read.
#[test]
fn the_write_classifier_tells_writes_from_reads() {
    let denied: BTreeSet<String> = db_writes::denied(&repo_root())
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    for write in [
        "oxplow_db::change_store::SqliteChangeStore::get_or_create",
        "oxplow_db::analytics_stores::SqliteUsageStore::record",
        "oxplow_domain::stores::ThreadStore::upsert",
        "oxplow_db::database::Database::transaction",
        "oxplow_db::database::Database::rehearse",
        "oxplow_app::exec_consent::ApprovalStore::approve",
    ] {
        assert!(denied.contains(write), "{write} isn't denied");
    }
    for read in [
        "oxplow_domain::stores::ThreadStore::get",
        "oxplow_domain::stores::ThreadStore::list_for_stream",
        "oxplow_db::database::Database::read",
        "oxplow_db::semantic_layer::SemanticLayer::query_sql",
    ] {
        assert!(!denied.contains(read), "{read} is denied");
    }
}

/// The marker a thin caller's off-bus write carries, its reason a row of
/// the doc's table (`ipc-and-stores.md` "What stays off the bus").
const OFF_BUS: &str = "#[expect(clippy::disallowed_methods, reason = \"off the bus: ";

/// tsk785, tsk903: a thin caller's write through a store is listed off
/// the bus where it is made — `#[expect]`, so it fails once it no longer
/// writes — with a reason the doc's table gives; nothing else lifts the
/// denial.
#[test]
fn thin_caller_off_bus_writes_say_why() {
    let doc = std::fs::read_to_string(repo_root().join(".context/ipc-and-stores.md")).unwrap();
    let mut offenders = Vec::new();
    let mut listed = 0;
    for (path, text) in production_sources() {
        if !thin_caller(&path) {
            continue;
        }
        for (i, _) in text.match_indices("disallowed_methods") {
            // The whole attribute, however rustfmt laid it out.
            let start = text[..i].rfind("#[").unwrap_or(i);
            let end = text[i..].find(")]").map_or(text.len(), |e| i + e + 2);
            let attr = text[start..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace("( ", "(")
                .replace(" )", ")");
            let reason = attr
                .strip_prefix(OFF_BUS)
                .and_then(|r| r.strip_suffix("\")]"));
            match reason {
                Some(r) if doc.contains(r) => listed += 1,
                _ => {
                    let line = text[..i].lines().count();
                    offenders.push(format!("{path}:{line}: {attr}"));
                }
            }
        }
    }
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "lift the denial only with `{OFF_BUS}<a row of the doc's off-bus table>\")]`"
    );
    assert!(listed > 0, "the scan saw no off-bus write");
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

/// What marks code kept for an old shape (tsk865, tsk920): a general
/// word, matched case-insensitively, and the names of shims already
/// removed, so neither comes back under any name.
const LEGACY_MARKERS: &[&str] = &[
    "legacy",
    "back-compat",
    "backcompat",
    "backward compat",
    "backwards compat",
    "backward-compat",
    "older shape",
    "old shape",
    "deprecated alias",
    "#[deprecated",
    "@deprecated",
    "compat shim",
    "for compatibility",
    "api compatibility",
    "old data",
    "migrate_v1",
    "migrate_gauges",
    "gauges_to_collectors",
    "gauges_retired",
    "plugin migrate",
    "hook_bridge.py",
];

/// Lines that carry a marker and stay, each with why: `(file, a piece of
/// the line, reason)`. A row that no longer matches is stale.
const LEGACY_KEPT: &[(&str, &str, &str)] = &[
    (
        "crates/oxplow-domain/src/snapshot.rs",
        "Legacy",
        "the `legacy` trigger V97 gave the snapshots taken before the op log: rows already in the database",
    ),
    (
        "crates/oxplow-domain/src/events/schema.rs",
        "Legacy,",
        "`snapshot.taken`'s trigger for those same V97 rows",
    ),
    (
        "apps/desktop/src/pages/LocalHistoryDashboardPage.tsx",
        "legacy: \"\"",
        "the label of that trigger",
    ),
    (
        "apps/desktop/src/components/Comments/selectors.ts",
        "egacy",
        "comment selectors stored as a per-surface position object: rows already in the database",
    ),
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

/// tsk865, tsk920: no production source keeps a shim, alias or migrator
/// for an old shape — a manifest without `manifest: 2`, a `gauges:`
/// block, an old slot name, an old impact kind are errors, not
/// conversions. Any line naming one (`LEGACY_MARKERS`) is either gone or
/// listed in `LEGACY_KEPT` with why it stays.
#[test]
fn no_legacy_shims() {
    let mut found = Vec::new();
    let mut used = BTreeSet::new();
    for (path, text) in production_sources().into_iter().chain(desktop_sources()) {
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            if !LEGACY_MARKERS.iter().any(|m| lower.contains(m)) {
                continue;
            }
            match LEGACY_KEPT
                .iter()
                .position(|(p, piece, _)| *p == path && line.contains(piece))
            {
                Some(i) => {
                    used.insert(i);
                }
                None => found.push(format!("{path}:{}: {}", n + 1, line.trim())),
            }
        }
    }
    assert_eq!(
        found,
        Vec::<String>::new(),
        "remove the old shape, or list the line in LEGACY_KEPT with why it stays"
    );
    let stale: Vec<_> = LEGACY_KEPT
        .iter()
        .enumerate()
        .filter(|(i, _)| !used.contains(i))
        .map(|(_, k)| k)
        .collect();
    assert!(
        stale.is_empty(),
        "LEGACY_KEPT rows that match nothing: {stale:?}"
    );
}

/// tsk942: an effort's close is reconciled in one place — the
/// effort-lifecycle consumer's `on_effort_closed` (`task_service.rs`),
/// on `effort.closed`. Nothing else (crash recovery, a command) runs the
/// reconcile itself.
#[test]
fn effort_close_reconciles_in_one_place() {
    let callers: Vec<String> = production_sources()
        .into_iter()
        .filter(|(_, text)| text.contains("reconcile_close("))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        callers,
        vec![
            "crates/oxplow-app/src/attribution.rs".to_string(),
            "crates/oxplow-app/src/task_service.rs".to_string(),
        ]
    );
    let recovery = production_sources()
        .into_iter()
        .find(|(path, _)| path == "crates/oxplow-app/src/recovery.rs")
        .expect("recovery.rs")
        .1;
    assert!(
        !recovery.contains("unattributed"),
        "recovery records no residue itself"
    );
}

/// The workspace's test doubles, by their names' convention: a crate
/// that stands in for a service in tests is `<name>-fake` or `<name>-sim`.
fn is_test_double(name: &str) -> bool {
    name.ends_with("-fake") || name.ends_with("-sim")
}

/// tsk930: a test double (the stand-in OAuth server signs anyone in; the
/// fakes answer whatever a test scripts) is a dev-dependency only — no
/// workspace crate reaches one through a normal or build edge of the
/// **resolved** graph (`cargo metadata`), however its manifest spells
/// the dependency (a `[dependencies.x]` table, a renamed `package =`, the
/// workspace-hack).
#[test]
fn no_test_double_is_a_production_dependency() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let name_of: std::collections::HashMap<&str, &str> = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap()))
        .collect();
    // Each package's normal and build edges.
    let edges: std::collections::HashMap<&str, Vec<&str>> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let shipped = n["deps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| {
                    d["dep_kinds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|k| k["kind"].as_str() != Some("dev"))
                })
                .map(|d| d["pkg"].as_str().unwrap())
                .collect();
            (n["id"].as_str().unwrap(), shipped)
        })
        .collect();
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    let doubles: BTreeSet<&str> = members
        .iter()
        .map(|m| name_of[m])
        .filter(|n| is_test_double(n))
        .collect();
    for double in [
        "oxplow-oauth-sim",
        "oxplow-ai-fake",
        "oxplow-provider-fake",
        "oxplow-acp-fake",
        // The browser suite's daemon: secrets in memory (tsk948).
        "oxplow-daemon-sim",
    ] {
        assert!(
            doubles.contains(double),
            "{double} isn't seen as a test double"
        );
    }
    let mut offenders = Vec::new();
    for member in members.iter().filter(|m| !is_test_double(name_of[*m])) {
        let mut seen = BTreeSet::new();
        let mut stack = vec![(*member, vec![name_of[member]])];
        while let Some((id, path)) = stack.pop() {
            for dep in edges.get(id).into_iter().flatten() {
                if !seen.insert(*dep) {
                    continue;
                }
                let mut via = path.clone();
                via.push(name_of[dep]);
                if doubles.contains(name_of[dep]) {
                    offenders.push(via.join(" → "));
                } else {
                    stack.push((dep, via));
                }
            }
        }
    }
    assert_eq!(offenders, Vec::<String>::new(), "a test double ships");
}

/// P10: a sign-in's redirect is caught by the desktop shell, never the
/// core — so it lands where the person's browser is, with a remote daemon
/// too. The listener is its own crate (`oxplow-oauth-redirect`, the
/// shell's), which no core crate uses; the providers bind no socket at
/// all.
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
        .filter(|(path, _)| CORE.iter().any(|c| path.starts_with(c)))
        .filter(|(path, text)| {
            text.contains("RedirectListener")
                || text.contains("oxplow_oauth_redirect")
                || (path.starts_with("crates/oxplow-app/src/providers/")
                    && text.contains("TcpListener"))
        })
        .map(|(path, _)| path)
        .collect();
    assert!(offenders.is_empty(), "{offenders:#?}");
}

/// tsk1018: there is no tmux in the app — agents and shells run as
/// direct PTYs. No tracked source, manifest, script or CI file names it.
/// Prose (`docs/`, `.context/`, `DEV.md`) may say it's gone or suggest
/// a person's own multiplexer; `.oxplow/` is this repo's config, not the
/// app; this guard names it to look for it.
#[test]
fn no_tmux_in_the_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    let named: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|path| {
            ["crates/", "apps/", "tests-e2e/", "scripts/", ".github/"]
                .iter()
                .any(|dir| path.starts_with(dir))
                || matches!(*path, "Cargo.toml" | "package.json")
        })
        .filter(|path| !path.ends_with("source_guards.rs"))
        .filter(|path| {
            std::fs::read(root.join(path)).is_ok_and(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .to_ascii_lowercase()
                    .contains("tmux")
            })
        })
        .map(str::to_string)
        .collect();
    assert_eq!(named, Vec::<String>::new());
}
