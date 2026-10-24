import { useCallback, useEffect, useState } from "react";

import { runLens } from "../api.js";
import { recordOpError } from "../opErrors.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { resolveLiveOrigin } from "../lens/linkOrigin.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { identityOf, type LiveOrigin, type Origin, type OriginItem } from "./session.js";
import type { TabRef } from "./tabState.js";

/**
 * The origin a page's Up and Down step through. A stored origin is its own;
 * a live one (a lens's link column) takes its items from `items`, the lens
 * run now, and its place from where `current` is in them. None until the
 * lens has run, and none when the page is no longer in it.
 */
export function resolvedOrigin(origin: Origin, items: OriginItem[] | undefined, current: TabRef): Origin | null {
  if (!origin.live) return origin;
  if (!items) return null;
  const page = identityOf(current.id);
  const index = items.findIndex((i) => identityOf(i.ref.id) === page);
  return index === -1 ? null : { ...origin, items, index };
}

/** The items resolved for `slot`'s live `origin`, if they are of that origin. */
export function liveItemsOf(
  resolved: Record<string, { key: string; items: OriginItem[] }>,
  slot: string,
  origin: Origin | undefined,
): OriginItem[] | undefined {
  const got = resolved[slot];
  return origin?.live && got?.key === JSON.stringify(origin.live) ? got.items : undefined;
}

/** Runs a live origin's lens, and again whenever what it read changes,
 *  handing the host its items. Draws nothing. */
export function LiveOriginResolver({ live, streamId, onItems }: { live: LiveOrigin; streamId: string | null; onItems(items: OriginItem[]): void }) {
  const [reads, setReads] = useState<Reads>(NO_READS);
  const key = JSON.stringify([live, streamId]);
  const load = useCallback(() => {
    let current = true;
    runLens(live.lens, live.params, streamId).then(
      (run) => {
        if (!current) return;
        setReads(run.result.reads);
        onItems(resolveLiveOrigin(run, live.column));
      },
      (e: unknown) => recordOpError({ label: "Up and Down", message: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useEffect(() => load(), [load]);
  useRerunOnChange(reads, () => void load());
  return null;
}
