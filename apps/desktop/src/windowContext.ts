/// The shell's window context, read apart from the transport so the
/// device id can use it without importing the API layer.

/// What the shell injects into every window it creates, before any page
/// script runs. Absent in a plain-browser session.
export type WindowContext = {
  base: string | null;
  /// The daemon's UI token (only this window's renderer gets it).
  token?: string | null;
  kind: string;
  projectDir: string | null;
  /// The host's id for the project, when a host served this page
  /// (`/p/<project>/`); the shell names a project by its folder instead.
  project?: string;
  /// The host's origin, when a host served this page: its API is there
  /// (pairing, the device's own token), and the page carries no token.
  host?: string;
  /// The Mac app's device: its id is kept by the shell across runs.
  device?: { id: string; name: string };
};

export function readWindowContext(): WindowContext | null {
  try {
    const injected = (globalThis as { __OXPLOW__?: WindowContext }).__OXPLOW__;
    if (injected && typeof injected === "object") return injected;
  } catch {
    // No window object (bun tests) — fall through.
  }
  return null;
}
