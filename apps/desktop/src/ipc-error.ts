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

/** The `code` of a command's error payload (`NEEDS_CONFIRMATION`,
 *  `PROPOSED` — an agent-driven run kept for a person's approval —
 *  `INVALID`, `DENIED`, …), or null for a string payload. */
export function ipcErrorCode(err: unknown): string | null {
  if (err && typeof err === "object") {
    const code = (err as { code?: unknown }).code;
    if (typeof code === "string" && code.trim()) return code;
  }
  return null;
}

/** A failed IPC call: its message, and its code when it has one, so a
 *  caller can tell "ask the person first" (`NEEDS_CONFIRMATION`) from a
 *  failure. */
export class IpcCallError extends Error {
  constructor(
    message: string,
    readonly code: string | null,
  ) {
    super(message);
    this.name = "IpcCallError";
  }
}

/** The command needs the person to confirm it; call again confirmed. */
export function needsConfirmation(e: unknown): boolean {
  return e instanceof IpcCallError && e.code === "NEEDS_CONFIRMATION";
}
