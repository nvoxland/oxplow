//! Tauri adapters generated from the shared command table.
//!
//! Every adapter in this crate used to be hand-written: three lines
//! delegating to an `oxplow_rpc` core, times two hundred. They exist
//! only so `tauri-specta` has something to derive the TS binding from —
//! nothing invokes them, because the renderer's transport routes
//! everything but the shell surface to the window's daemon.
//!
//! So they're generated. `oxplow_rpc::oxplow_command_table!` holds each
//! command once — wire name, core, argument names and types, return
//! type — and expands here into the adapter and in `oxplow-rpc` into
//! the dispatch arm. Adding a command is one table row.
//!
//! The adapters keep `State<'_, AppState>` because that is what makes
//! them Tauri commands with the right shape; in the shell process no
//! `AppState` is managed, and nothing can reach them to find out.

/// Emit one `#[tauri::command]` adapter per `gen` row. The `ctx` and
/// `svc` sections are ignored here — those commands still have
/// hand-written adapters, and the migration moves rows from `svc` to
/// `gen` a module at a time.
macro_rules! tauri_adapters {
    (
        ctx { $( $cname:literal => $ccore:path { $( $cfield:ident : $cfty:ty ),* $(,)? } ),* $(,)? }
        svc { $( $name:literal => $core:path { $( $field:ident : $fty:ty ),* $(,)? } ),* $(,)? }
        gen { $( $gname:ident => $gcore:path { $( $gfield:ident : $gfty:ty ),* $(,)? } -> $gret:ty ),* $(,)? }
    ) => {
        $(
            /// Generated from the command table in `oxplow-rpc`; the
            /// implementation and its docs live on the core.
            // Flat positional params mirror the wire contract —
            // tauri-specta binds arguments by name, so bundling them
            // into a struct would change the renderer's call shape.
            #[allow(clippy::too_many_arguments)]
            #[tauri::command]
            #[specta::specta]
            pub async fn $gname(
                state: tauri::State<'_, crate::AppState>,
                $( $gfield : $gfty ),*
            ) -> Result<$gret, crate::IpcError> {
                $gcore(&state $(, $gfield )*).await
            }
        )*
    };
}

oxplow_rpc::oxplow_command_table!(tauri_adapters);
