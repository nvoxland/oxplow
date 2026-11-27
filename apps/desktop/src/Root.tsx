import { App } from "./App.js";
import { Launcher } from "./launcher/Launcher.js";
import { ProjectSetup } from "./launcher/ProjectSetup.js";
import { HostHome } from "./host/HostHome.js";
import { PairScreen } from "./host/PairScreen.js";
import { needsPairing, windowKind, windowProjectDir } from "./tauri-bridge/transport.js";

/// Top-level gate: which screen this window is.
///
/// The shell states it in `window.__OXPLOW__` before any script runs
/// (see `src-tauri/src/windows.rs`), so there is nothing to ask and
/// nothing to wait for — no loading flash, and no IPC round trip before
/// the app can decide what it is.
///
/// A page a host served (`/p/<project>/`, `/`) says so too: its home is
/// `hostHome`, and a project page on a browser with no token for that host
/// pairs first, before the app mounts (the transport reads its token once,
/// at load).
///
/// A window the shell didn't create — a plain browser driving a daemon
/// over a tunnel — has no context and gets the app shell, the only
/// screen that means anything there.
export function Root() {
  const kind = windowKind();
  if (kind === "launcher") return <Launcher />;
  if (kind === "setup") return <ProjectSetup dir={windowProjectDir() ?? ""} />;
  if (kind === "hostHome") return <HostHome />;
  if (needsPairing()) return <PairScreen />;
  return <App />;
}
