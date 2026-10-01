import { useEffect, useState } from "react";

import { listExtensions, subscribeOxplowEvents } from "../api.js";
import { extensionsChanged } from "./lensRerun.js";
import { slotExtensions } from "./lensModel.js";

/** Whether any enabled extension mounts a lens in `slot` — for a page
 *  that gives a slot a region of its own (a side column) only when
 *  something fills it. */
export function useSlotMounted(slot: string, streamId: string | null): boolean {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    let live = true;
    const load = () =>
      listExtensions(streamId)
        .then((exts) => {
          if (live) setMounted(slotExtensions(exts, slot).length > 0);
        })
        .catch(() => {
          if (live) setMounted(false);
        });
    load();
    const off = subscribeOxplowEvents((e) => {
      if (extensionsChanged(e as Record<string, unknown>)) load();
    });
    return () => {
      live = false;
      off();
    };
  }, [slot, streamId]);
  return mounted;
}
