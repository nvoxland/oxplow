import { useCallback, useEffect, useMemo, useState } from "react";

import { querySql } from "../api.js";
import { useExtensions } from "../extensionsStore.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import type { DecoratorPlacement, Reads } from "../tauri-bridge/generated/bindings.js";
import { decorationQueries, decorationsFromResult, decoratorsFor, type Decoration } from "./decorators.js";

/** The decorations for `refs` in `placement`: each decorator's queries,
 *  re-run when a model it read changes or the extensions do. A decorator
 *  whose query fails shows nothing — decorations are additive. An answer
 *  for inputs that have since changed is dropped. */
export function useDecorations(placement: DecoratorPlacement, refs: string[], streamId: string | null): Decoration[] {
  const [decorations, setDecorations] = useState<Decoration[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const exts = useExtensions(streamId);
  const decorators = useMemo(() => decoratorsFor(exts ?? [], placement), [exts, placement]);
  const refsKey = JSON.stringify([...new Set(refs)].sort());
  const guard = useRequestGuard();
  const load = useCallback(async () => {
    const current = guard.begin();
    const wanted = JSON.parse(refsKey) as string[];
    if (wanted.length === 0 || decorators.length === 0) {
      setDecorations([]);
      setReads(NO_READS);
      return;
    }
    const results = await Promise.all(
      decorators.flatMap((d) =>
        decorationQueries(d, wanted).map(async (q) => {
          try {
            const res = await querySql(q.sql, q.params, q.limit);
            return { decorations: decorationsFromResult(res, d.extension), reads: res.reads };
          } catch {
            return null;
          }
        }),
      ),
    );
    if (!current()) return;
    setDecorations(results.flatMap((r) => r?.decorations ?? []));
    setReads(unionReads(results.map((r) => r?.reads)));
  }, [decorators, refsKey, guard]);
  useEffect(() => void load(), [load]);
  useRerunOnChange(reads, () => void load());
  return decorations;
}
