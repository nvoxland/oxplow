// Native dialogs, behind the transport facade.
//
// The OS folder picker is inherently shell-local: it can only run when
// a Tauri host is present. UI code must reach it through here rather
// than importing `@tauri-apps/plugin-dialog` directly, so that "all
// native access funnels through tauri-bridge/" stays an enforceable
// invariant (see no-tauri-imports.test.ts). In a plain-browser session
// (no Tauri host — Playwright-driven UX testing, a served `dist/`) the
// picker can't open, so it resolves to `null` instead of throwing.

import { open as tauriOpen } from "@tauri-apps/plugin-dialog";
import { shellAvailable } from "./transport";

/// Open the native single-folder picker. Resolves with the selected
/// absolute path, or `null` if the user cancels or no Tauri host is
/// present (plain browser).
export async function pickFolder(title: string): Promise<string | null> {
  if (!shellAvailable()) return null;
  const selected = await tauriOpen({ directory: true, multiple: false, title });
  return typeof selected === "string" ? selected : null;
}
