import { useAppPreferenceValue } from "../prefs/remembered.js";
import { getDevice, type Device } from "../device.js";
import { readWindowContext } from "../windowContext.js";
import { SHELL_CONTRIBUTIONS, SHELL_PLACES } from "./shellContract.js";

/** The shells that draw a window: the Mac's desktop layout, and the tablet's
 *  (one page at full width, a switcher, a panel tray). A phone gets the
 *  tablet's until it has its own. */
export type ShellKind = "desktop" | "tablet";

const SHELLS: readonly ShellKind[] = ["desktop", "tablet"];

function asShell(v: unknown): ShellKind | null {
  return typeof v === "string" && (SHELLS as readonly string[]).includes(v) ? (v as ShellKind) : null;
}

/** Whether a shell draws every contribution (`shellContract.ts`): only then
 *  is it a device's default, so no device lands in a shell that hides a
 *  feature. A shell with gaps is drawn only when chosen. */
export function shellComplete(shell: ShellKind): boolean {
  return SHELL_CONTRIBUTIONS.every((c) => SHELL_PLACES[shell][c] !== null);
}

/** Which shell draws the window: an override (a test's), else the device's
 *  own choice (its `shell` app preference), else its kind's default (the
 *  tablet shell for a tablet or phone, once it is complete). Pure. */
export function chooseShell(src: {
  override: unknown;
  preference: unknown;
  deviceKind: Device["kind"];
  complete?: (shell: ShellKind) => boolean;
}): ShellKind {
  const chosen = asShell(src.override) ?? asShell(src.preference);
  if (chosen) return chosen;
  return src.deviceKind !== "desktop" && (src.complete ?? shellComplete)("tablet") ? "tablet" : "desktop";
}

/** A test's override: the page's `?shell=` or the window context's `shell`. */
export function shellOverride(search: string, context: { shell?: unknown } | null): ShellKind | null {
  return asShell(new URLSearchParams(search).get("shell")) ?? asShell(context?.shell);
}

/** The window's shell, following the device's preference as it loads (a
 *  narrow read: the window re-renders only when the choice changes). */
export function useShellKind(): ShellKind {
  const preference = useAppPreferenceValue("shell");
  const search = typeof location === "undefined" ? "" : location.search;
  return chooseShell({
    override: shellOverride(search, readWindowContext() as { shell?: unknown } | null),
    preference,
    deviceKind: getDevice().kind,
  });
}
