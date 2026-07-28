import { App } from "./App.js";
import { Launcher } from "./launcher/Launcher.js";
import { ProjectSetup } from "./launcher/ProjectSetup.js";
import { windowKind, windowProjectDir } from "./tauri-bridge/transport.js";

/// Top-level gate: which screen this window is.
///
/// The shell states it in `window.__OXPLOW__` before any script runs
/// (see `src-tauri/src/windows.rs`), so there is nothing to ask and
/// nothing to wait for — no loading flash, and no IPC round trip before
/// the app can decide what it is.
///
/// A window the shell didn't create — a plain browser driving a daemon
/// over a tunnel — has no context and gets the app shell, the only
/// screen that means anything there.
export function Root() {
  const kind = windowKind();
  if (kind === "launcher") return <Launcher />;
  if (kind === "setup") return <ProjectSetup dir={windowProjectDir() ?? ""} />;
  return <App />;
}
