/**
 * An extension's problems as a person reads them (`.context/extensions-sdk.md`
 * "Diagnostics"). A diagnostic's `file` is relative to the extension's
 * folder; it is shown under the folder they edit — its source, never the
 * copy that runs. The same line the backend's `Diagnostic::render` writes.
 */
import type { Diagnostic, Extension } from "./tauri-bridge/generated/bindings.js";

/** `<root>/<file>:<line>: <message> — fix: <fix>`; without a file, the message. */
export function renderDiagnostic(d: Diagnostic, root: string): string {
  const base = root.replace(/\/+$/, "");
  let out = d.message;
  if (d.file !== null) {
    const path = base === "" ? d.file : `${base}/${d.file}`;
    out = d.line !== null ? `${path}:${d.line}: ${d.message}` : `${path}: ${d.message}`;
  }
  return d.fix !== null ? `${out} — fix: ${d.fix}` : out;
}

/** Its errors as lines, under its source folder. */
export function shownErrors(ext: Extension): string[] {
  const root = ext.source ?? ext.path;
  return ext.errors.map((d) => renderDiagnostic(d, root));
}
