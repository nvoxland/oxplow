import { useAppPreference } from "../prefs/remembered.js";
import { getDevice, type Device } from "../device.js";
import { readWindowContext } from "../windowContext.js";

/** The shells that draw a window: the Mac's desktop layout, and the tablet's
 *  (one page at full width, a switcher, a panel tray). A phone gets the
 *  tablet's until it has its own. */
export type ShellKind = "desktop" | "tablet";

const SHELLS: readonly ShellKind[] = ["desktop", "tablet"];

function asShell(v: unknown): ShellKind | null {
  return typeof v === "string" && (SHELLS as readonly string[]).includes(v) ? (v as ShellKind) : null;
}

/** Which shell draws the window: an override (a test's), else the device's
 *  own choice (its `shell` app preference), else its kind's default. Pure. */
export function chooseShell(src: { override: unknown; preference: unknown; deviceKind: Device["kind"] }): ShellKind {
  return asShell(src.override) ?? asShell(src.preference) ?? (src.deviceKind === "desktop" ? "desktop" : "tablet");
}

/** A test's override: the page's `?shell=` or the window context's `shell`
 *  (the e2e harness injects it). */
export function shellOverride(search: string, context: { shell?: unknown } | null): ShellKind | null {
  return asShell(new URLSearchParams(search).get("shell")) ?? asShell(context?.shell);
}

/** The window's shell, following the device's preference as it loads. */
export function useShellKind(): ShellKind {
  const [preference] = useAppPreference<string | null>("shell", null);
  const search = typeof location === "undefined" ? "" : location.search;
  return chooseShell({
    override: shellOverride(search, readWindowContext() as { shell?: unknown } | null),
    preference,
    deviceKind: getDevice().kind,
  });
}
