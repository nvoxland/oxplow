//! The frontend-facing IPC result envelope — a single Rust owner.
//!
//! Every command result the renderer sees is shaped
//! `{"status":"ok","data":…}` or `{"status":"error","error":<IpcError>}`.
//! That shape has two producers by mechanism:
//!
//! - **Daemon** (`oxplow-daemon`) — serializes a `dispatch()` result
//!   into JSON in its `/ipc/:name` route. That serialization lives
//!   here, in [`ipc_envelope`], so the daemon can't hand-roll a
//!   divergent shape.
//! - **Tauri shell** — the `#[tauri::command]` wrappers return
//!   `Result<T, IpcError>`; Tauri's invoke protocol resolves/rejects,
//!   and the generated `typedError` TS wrapper
//!   (`apps/desktop/src/tauri-bridge/generated/bindings.ts`) folds that
//!   into the *same* `{status, data|error}` shape. It builds the
//!   envelope in TypeScript, not Rust, so it can't literally call this
//!   function — but it encodes the byte-identical shape, and crucially
//!   the `error` branch serializes the **same** [`IpcError`] type
//!   (oxplow-tauri-ipc re-exports `oxplow_rpc::IpcError`), so the two
//!   producers cannot diverge on error mapping or field naming.
//!
//! [`ENVELOPE_CONTRACT`] documents the keys both sides must agree on;
//! the tests below pin them.

use serde_json::{json, Value};

use crate::IpcError;

/// The envelope keys both hosts must agree on, mirrored by the TS
/// `typedError` wrapper. Referenced by name so a future rename here
/// forces a deliberate edit there (and vice versa).
pub const ENVELOPE_CONTRACT: &str = "{status:\"ok\",data} | {status:\"error\",error}";

/// Wrap a [`dispatch`](crate::dispatch) result into the frontend-facing
/// IPC envelope. The daemon's HTTP route calls this directly; see the
/// module docs for why the Tauri path reaches the identical shape
/// without calling it.
pub fn ipc_envelope(result: Result<Value, IpcError>) -> Value {
    match result {
        Ok(data) => json!({ "status": "ok", "data": data }),
        Err(error) => json!({ "status": "error", "error": error }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_branch_matches_typed_error_shape() {
        // typedError returns { status: "ok", data: <T> }.
        let env = ipc_envelope(Ok(json!("pong")));
        assert_eq!(env, json!({ "status": "ok", "data": "pong" }));
    }

    #[test]
    fn ok_branch_preserves_null_data() {
        // Many commands resolve to `null` (TS `typedError<null, _>`);
        // the key must still be present and explicitly null.
        let env = ipc_envelope(Ok(Value::Null));
        assert_eq!(env, json!({ "status": "ok", "data": null }));
        assert!(env.get("data").is_some(), "data key must be present");
    }

    #[test]
    fn error_branch_serializes_ipc_error_camelcase() {
        // typedError returns { status: "error", error: <IpcError> }.
        // IpcError serializes camelCase with code/message/cause.
        let env = ipc_envelope(Err(IpcError::not_found()));
        assert_eq!(
            env,
            json!({
                "status": "error",
                "error": { "code": "NOT_FOUND", "message": "not found", "cause": null }
            })
        );
    }

    #[test]
    fn error_branch_is_byte_identical_to_direct_ipc_error_serialization() {
        // The Tauri reject path serializes the *same* IpcError directly
        // (Tauri core + typedError), so the `error` member here must be
        // byte-for-byte what `serde_json` produces for that IpcError.
        let err = IpcError::invalid("bad input").with_cause("inner detail");
        let env = ipc_envelope(Err(err.clone()));
        assert_eq!(env["error"], serde_json::to_value(&err).unwrap());
    }
}
