//! Enforces that the Tauri IPC surface (UI) and the MCP surface (agent) stay
//! in sync with the manifest in `oxplow_surface_parity`. See that crate's
//! module docs for the four exposures and the `AgentTodo → Both` ratchet.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use oxplow_surface_parity::{manifest_shape_errors, Exposure, MANIFEST};

/// A throwaway `tauri_specta` export target that, instead of writing a file,
/// captures the registered command names off the builder configuration. This
/// is the only way to read the (otherwise private) command list out of a
/// `tauri_specta::Builder` without booting Tauri.
struct NameSink(RefCell<Vec<String>>);

impl tauri_specta::LanguageExt for &NameSink {
    type Error = std::io::Error;

    fn export(
        self,
        cfg: &tauri_specta::BuilderConfiguration,
        _path: &Path,
    ) -> Result<(), std::io::Error> {
        *self.0.borrow_mut() = cfg.commands.iter().map(|f| f.name().to_string()).collect();
        Ok(())
    }
}

/// The IPC command names actually registered via `collect_commands!`.
fn ipc_command_names() -> BTreeSet<String> {
    let sink = NameSink(RefCell::new(Vec::new()));
    oxplow_tauri_ipc::specta_builder()
        // The path is ignored by NameSink::export — nothing is written.
        .export(&sink, Path::new("parity-name-sink-unused"))
        .expect("NameSink export is infallible");
    sink.0.into_inner().into_iter().collect()
}

/// The MCP tool names actually registered via `#[tool]`.
fn mcp_tool_names() -> BTreeSet<String> {
    oxplow_mcp::registered_tool_names().into_iter().collect()
}

#[test]
fn surface_parity() {
    // The manifest must be internally well-formed before comparing surfaces.
    let shape = manifest_shape_errors();
    assert!(
        shape.is_empty(),
        "manifest shape errors:\n{}",
        shape.join("\n")
    );

    let ipc = ipc_command_names();
    let mcp = mcp_tool_names();

    let ipc_in_manifest: BTreeSet<&str> = MANIFEST.iter().filter_map(|c| c.ipc).collect();
    let mcp_in_manifest: BTreeSet<&str> = MANIFEST.iter().filter_map(|c| c.mcp).collect();

    let mut problems: Vec<String> = Vec::new();

    // (a) Every actually-registered op has a manifest row, and no row names a
    //     command/tool that no longer exists.
    let unclassified_ipc: Vec<&str> = ipc
        .iter()
        .map(String::as_str)
        .filter(|n| !ipc_in_manifest.contains(n))
        .collect();
    if !unclassified_ipc.is_empty() {
        problems.push(format!(
            "IPC commands with no manifest row — classify each in MANIFEST \
             (Both/UiOnly/AgentTodo): {unclassified_ipc:?}"
        ));
    }
    let unclassified_mcp: Vec<&str> = mcp
        .iter()
        .map(String::as_str)
        .filter(|n| !mcp_in_manifest.contains(n))
        .collect();
    if !unclassified_mcp.is_empty() {
        problems.push(format!(
            "MCP tools with no manifest row — classify each in MANIFEST \
             (Both/AgentOnly, or flip an AgentTodo row): {unclassified_mcp:?}"
        ));
    }
    let dangling_ipc: Vec<&str> = ipc_in_manifest
        .iter()
        .copied()
        .filter(|n| !ipc.contains(*n))
        .collect();
    if !dangling_ipc.is_empty() {
        problems.push(format!(
            "manifest names IPC commands that aren't registered (rename/remove the row): {dangling_ipc:?}"
        ));
    }
    let dangling_mcp: Vec<&str> = mcp_in_manifest
        .iter()
        .copied()
        .filter(|n| !mcp.contains(*n))
        .collect();
    if !dangling_mcp.is_empty() {
        problems.push(format!(
            "manifest names MCP tools that aren't registered (rename/remove the row): {dangling_mcp:?}"
        ));
    }

    // (b) Both rows present on both surfaces; (c) UiOnly/AgentOnly don't leak.
    for c in MANIFEST {
        match c.exposure {
            Exposure::Both => {
                let (i, m) = (c.ipc.unwrap(), c.mcp.unwrap());
                if !ipc.contains(i) {
                    problems.push(format!("Both `{}` missing IPC command `{i}`", c.capability));
                }
                if !mcp.contains(m) {
                    problems.push(format!("Both `{}` missing MCP tool `{m}`", c.capability));
                }
            }
            Exposure::UiOnly => {
                // A leak is a tool that shares this command's name AND isn't
                // already claimed by another row's `mcp` — the latter guards
                // legitimate cross-surface name collisions (e.g. IPC
                // `delete_wiki_page` deletes a page; the MCP tool of the same
                // name deletes a note and is claimed by the `note.delete` row).
                let i = c.ipc.unwrap();
                if mcp.contains(i) && !mcp_in_manifest.contains(i) {
                    problems.push(format!(
                        "UiOnly `{}` leaked onto MCP as `{i}` — reclassify as Both",
                        c.capability
                    ));
                }
            }
            Exposure::AgentOnly => {
                let m = c.mcp.unwrap();
                if ipc.contains(m) && !ipc_in_manifest.contains(m) {
                    problems.push(format!(
                        "AgentOnly `{}` leaked onto IPC as `{m}` — reclassify as Both",
                        c.capability
                    ));
                }
            }
            Exposure::AgentTodo => {
                let i = c.ipc.unwrap();
                if !ipc.contains(i) {
                    problems.push(format!(
                        "AgentTodo `{}` names IPC command `{i}` that isn't registered",
                        c.capability
                    ));
                }
            }
        }
    }

    // (d) Print the tracked gap backlog (visible under `--nocapture` / CI logs).
    let mut backlog: Vec<&str> = MANIFEST
        .iter()
        .filter(|c| c.exposure == Exposure::AgentTodo)
        .filter_map(|c| c.ipc)
        .collect();
    backlog.sort_unstable();
    println!(
        "\n=== MCP parity backlog: {} AgentTodo gaps ===",
        backlog.len()
    );
    for name in &backlog {
        println!("  {name}");
    }
    println!("=== end MCP parity backlog ===\n");

    assert!(
        problems.is_empty(),
        "surface parity violations ({} actual IPC / {} actual MCP):\n{}",
        ipc.len(),
        mcp.len(),
        problems.join("\n")
    );
}

/// The shell surface — commands that exist only as Tauri adapters. Defined in
/// `oxplow_tauri_ipc` (the crate that owns the adapters) because the renderer's
/// transport needs it too, generated into
/// `apps/desktop/src/tauri-bridge/generated/shellCommands.ts`. See
/// `shell_command_table_matches_the_rust_definition` below.
const TAURI_ONLY: &[&str] = oxplow_tauri_ipc::SHELL_ONLY_COMMANDS;

/// Every Tauri IPC command (the renderer's full surface) must be routable in
/// remote-daemon mode — i.e. present in the shared `oxplow_rpc::dispatch`
/// registry — unless it is on the explicit `TAURI_ONLY` allowlist. Without this
/// guard a command can be wired as a `#[tauri::command]` (and get TS bindings)
/// yet be forgotten in `rpc_dispatch!`, silently 404ing for remote users. (This
/// is exactly how the metrics effort-band / findings / override / scaffold
/// commands shipped broken before tsk236.)
#[test]
fn dispatch_registry_covers_ipc_surface() {
    let ipc = ipc_command_names();
    let dispatch: BTreeSet<&str> = oxplow_rpc::registered_command_names()
        .iter()
        .copied()
        .collect();
    let allow: BTreeSet<&str> = TAURI_ONLY.iter().copied().collect();

    // (a) Every IPC command is either dispatchable or explicitly Tauri-only.
    let missing: Vec<&str> = ipc
        .iter()
        .map(String::as_str)
        .filter(|n| !dispatch.contains(n) && !allow.contains(n))
        .collect();
    assert!(
        missing.is_empty(),
        "IPC commands wired as Tauri adapters but missing from the \
         oxplow_rpc::dispatch registry (they 404 in remote-daemon mode) — \
         add each to rpc_dispatch! in crates/oxplow-rpc/src/lib.rs, or add to \
         TAURI_ONLY if remote mode genuinely can't serve it: {missing:?}"
    );

    // (b) Keep the allowlist honest: no entry that is actually dispatchable,
    //     and no entry that no longer names a real IPC command.
    let stale_allow: Vec<&str> = TAURI_ONLY
        .iter()
        .copied()
        .filter(|n| dispatch.contains(n) || !ipc.contains(*n))
        .collect();
    assert!(
        stale_allow.is_empty(),
        "TAURI_ONLY names commands that are either now dispatchable or no \
         longer registered — prune them: {stale_allow:?}"
    );
}

/// The renderer's event-channel registry
/// (`apps/desktop/src/tauri-bridge/channels.ts`) must mirror
/// `oxplow_app::event_channels` exactly: same frame keys, same channel
/// names, no extras on either side. The daemon frames `/events` with
/// the Rust side; the renderer demuxes with the TS side — drift means
/// a whole event channel silently goes dark in remote mode.
#[test]
fn event_channels_match_typescript() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let channels_ts = Path::new(&manifest_dir)
        .join("../../apps/desktop/src/tauri-bridge/channels.ts")
        .canonicalize()
        .expect("channels.ts exists");
    let body = std::fs::read_to_string(&channels_ts).expect("read channels.ts");

    // Extract `key: "value",` pairs from the EVENT_CHANNELS literal.
    let block = body
        .split("export const EVENT_CHANNELS = {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("EVENT_CHANNELS literal in channels.ts");
    let mut ts_pairs: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        ts_pairs.push((key.trim().to_string(), value.to_string()));
    }
    ts_pairs.sort();

    let mut rust_pairs: Vec<(String, String)> = oxplow_app::event_channels::FRAMES
        .iter()
        .map(|(k, c)| (k.to_string(), c.to_string()))
        .collect();
    rust_pairs.sort();

    assert_eq!(
        ts_pairs, rust_pairs,
        "event-channel registries drifted — update \
         crates/oxplow-app/src/events.rs (event_channels) and \
         apps/desktop/src/tauri-bridge/channels.ts together"
    );
}

/// The renderer's transport splits commands two ways: shell commands go to
/// Tauri IPC, everything else to the window's daemon. That table is generated
/// from `oxplow_tauri_ipc::SHELL_ONLY_COMMANDS` by the `export_ts_bindings`
/// test, and this guard asserts the committed file still matches — a
/// hand-edited or stale `shellCommands.ts` would route a command to a backend
/// that doesn't serve it, which fails at runtime and nowhere else.
#[test]
fn shell_command_table_matches_the_rust_definition() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let generated = Path::new(&manifest_dir)
        .join("../../apps/desktop/src/tauri-bridge/generated/shellCommands.ts")
        .canonicalize()
        .expect("shellCommands.ts exists — run `cargo test -p oxplow-tauri-ipc`");
    let body = std::fs::read_to_string(&generated).expect("read shellCommands.ts");

    let block = body
        .split("export const SHELL_COMMANDS = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("SHELL_COMMANDS literal in shellCommands.ts");
    let mut ts: Vec<String> = block
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"'))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    ts.sort();

    let mut rust: Vec<String> = oxplow_tauri_ipc::SHELL_ONLY_COMMANDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    rust.sort();

    assert_eq!(
        ts, rust,
        "the generated shell-command table drifted from SHELL_ONLY_COMMANDS — \
         re-run `cargo test -p oxplow-tauri-ipc export_ts_bindings` and commit \
         apps/desktop/src/tauri-bridge/generated/shellCommands.ts"
    );
}
