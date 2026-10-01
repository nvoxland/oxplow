import { useEffect, useState } from "react";

import { listExtensions, subscribeOxplowEvents } from "../api.js";
import { extensionsChanged } from "../lens/lensRerun.js";
import type { UiCommand } from "../tauri-bridge/generated/bindings.js";

/** The enabled extensions' `ui.commands` for `streamId`, reloaded when
 *  what the extensions contribute changes. */
export function useUiCommands(streamId: string | null): UiCommand[] {
  const [commands, setCommands] = useState<UiCommand[]>([]);
  useEffect(() => {
    let live = true;
    const load = () =>
      listExtensions(streamId)
        .then((exts) => {
          if (live) setCommands(exts.filter((e) => e.enabled).flatMap((e) => e.ui.commands));
        })
        .catch(() => {
          // No extensions, no commands; the menus still work.
        });
    load();
    const off = subscribeOxplowEvents((e) => {
      if (extensionsChanged(e as Record<string, unknown>)) load();
    });
    return () => {
      live = false;
      off();
    };
  }, [streamId]);
  return commands;
}
