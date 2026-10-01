import { useCallback, useEffect, useState } from "react";

import { listExtensions, querySql, subscribeOxplowEvents } from "../api.js";
import { extensionsChanged, NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import type { DecoratorPlacement, Reads } from "../tauri-bridge/generated/bindings.js";
import { decorationQuery, decorationsFromResult, decoratorsFor, type Decoration } from "./decorators.js";

/** The decorations for `refs` in `placement`: one query per decorator,
 *  re-run when a model it read changes or the extensions do. A decorator
 *  whose query fails shows nothing — decorations are additive. */
export function useDecorations(placement: DecoratorPlacement, refs: string[], streamId: string | null): Decoration[] {
  const [decorations, setDecorations] = useState<Decoration[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const refsKey = JSON.stringify([...new Set(refs)].sort());
  const load = useCallback(async () => {
    const wanted = JSON.parse(refsKey) as string[];
    if (wanted.length === 0) {
      setDecorations([]);
      return;
    }
    const decorators = decoratorsFor(await listExtensions(streamId).catch(() => []), placement);
    const results = await Promise.all(
      decorators.map(async (d) => {
        const q = decorationQuery(d, wanted);
        if (!q) return null;
        try {
          const res = await querySql(q.sql, q.params, 1_000);
          return { decorations: decorationsFromResult(res, d.extension), reads: res.reads };
        } catch {
          return null;
        }
      }),
    );
    setDecorations(results.flatMap((r) => r?.decorations ?? []));
    setReads(unionReads(results.map((r) => r?.reads)));
  }, [placement, refsKey, streamId]);
  useEffect(() => {
    void load();
    return subscribeOxplowEvents((e) => {
      if (extensionsChanged(e as Record<string, unknown>)) void load();
    });
  }, [load]);
  useRerunOnChange(reads, () => void load());
  return decorations;
}
