import { useEffect, useState } from "react";

import { App } from "./App.js";
import { Launcher } from "./launcher/Launcher.js";
import { ProjectSetup } from "./launcher/ProjectSetup.js";
import { getLaunchMode } from "./api.js";
import { isRemote } from "./tauri-bridge/transport.js";
import { logUi } from "./logger.js";

type Screen =
  | { kind: "loading" }
  | { kind: "launcher" }
  | { kind: "setup"; dir: string }
  | { kind: "project" };

/// Top-level gate. Asks the backend which mode this process booted in
/// (launcher vs. project — see the process-per-window model in
/// `.context/architecture.md`) and renders the matching screen. The
/// full app shell (`<App>`) is unchanged for the project path.
export function Root() {
  const [screen, setScreen] = useState<Screen>({ kind: "loading" });

  useEffect(() => {
    // Remote mode: this window is a thin client to an oxplow-daemon
    // that owns the project — the LOCAL process's launch mode (usually
    // "launcher", since there's no local project) is irrelevant.
    // get_launch_mode is also a local-only command, so skip it.
    if (isRemote()) {
      setScreen({ kind: "project" });
      return;
    }
    getLaunchMode()
      .then((info) => {
        if (info.mode === "launcher") setScreen({ kind: "launcher" });
        else if (info.mode === "setup") setScreen({ kind: "setup", dir: info.projectDir ?? "" });
        else setScreen({ kind: "project" });
      })
      .catch((e) => {
        // get_launch_mode is managed in every mode, so this shouldn't
        // happen; fall through to the app shell, which has its own
        // daemon-down handling.
        logUi("error", "failed to resolve launch mode; assuming project", { error: String(e) });
        setScreen({ kind: "project" });
      });
  }, []);

  if (screen.kind === "loading") return null;
  if (screen.kind === "launcher") return <Launcher />;
  if (screen.kind === "setup") return <ProjectSetup dir={screen.dir} />;
  return <App />;
}
