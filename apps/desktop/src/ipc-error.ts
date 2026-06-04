/**
 * Reduce a Tauri command's error payload to a human-readable string.
 *
 * The tauri-specta envelope's `error` is usually an `IpcError` object
 * (`{ message, code }`), but arg-deserialization failures and panics arrive
 * as a plain **string**. The old `unwrap` only read `.message`/`.code`, so a
 * string error collapsed to the opaque literal "ipc error" — swallowing the
 * real reason (e.g. "invalid type: integer, expected a string") and making
 * bugs like the effort-diff snapshot-id mismatch much harder to diagnose.
 *
 * Order: a string is the reason; otherwise `message`, then `code`, then a
 * JSON dump of the payload, and only then the generic literal.
 */
export function ipcErrorMessage(err: unknown): string {
  if (typeof err === "string") return err.trim() || "ipc error";
  if (err && typeof err === "object") {
    const o = err as { message?: unknown; code?: unknown };
    if (typeof o.message === "string" && o.message.trim()) return o.message;
    if (typeof o.code === "string" && o.code.trim()) return o.code;
    try {
      const json = JSON.stringify(err);
      if (json && json !== "{}") return json;
    } catch {
      // Non-serializable (cycles, etc.) — fall through to the generic literal.
    }
  }
  return "ipc error";
}
