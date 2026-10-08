import { useEffect, useState } from "react";

import { listAgentHarnesses, onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { logUi } from "./logger.js";
import type { HarnessListing } from "./tauri-bridge/generated/bindings.js";

/// The registered agent harnesses in priority order (`agentKinds.ts`),
/// re-read when the config changes (`agents:` decides which are enabled
/// and their order) or the extensions do. Empty until loaded.
export function useAgentHarnesses(): HarnessListing[] {
  const [harnesses, setHarnesses] = useState<HarnessListing[]>([]);
  useEffect(() => {
    let cancelled = false;
    const reload = () => {
      void listAgentHarnesses()
        .then((next) => {
          if (!cancelled) setHarnesses(next);
        })
        .catch((error) => logUi("warn", "failed to list agent harnesses", { error: String(error) }));
    };
    reload();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") reload();
    });
    const unsubReconnect = onRemoteReconnect(reload);
    return () => {
      cancelled = true;
      unsub();
      unsubReconnect();
    };
  }, []);
  return harnesses;
}
