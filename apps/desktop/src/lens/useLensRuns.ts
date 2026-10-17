import { useCallback, useEffect, useState } from "react";

import { getLens, runLens, type LensRun, type SqlCell } from "../api.js";
import { useRequestGuard } from "../request-guard.js";
import { childParams } from "./lensModel.js";
import { unionReads, useRerunOnChange } from "./lensRerun.js";

/** One lens a page runs as a part of it: its key in the page, and the
 *  page's params (it takes the ones it declares). */
export interface PartLens {
  key: string;
  lensId: string;
}

export interface PartRun {
  run: LensRun | null;
  error: string | null;
}

/**
 * Runs the lenses that make up a page (a document's sections, a
 * dashboard's tiles, what extensions mount into a core page), each with
 * the params it declares from `params`, in `streamId`; all re-run when
 * anything one of them read changes. `null` until the first answer.
 */
export function useLensRuns(
  parts: readonly PartLens[],
  params: Record<string, SqlCell>,
  streamId: string | null,
): Map<string, PartRun> | null {
  const [runs, setRuns] = useState<Map<string, PartRun> | null>(null);
  const guard = useRequestGuard();
  const partsKey = parts.map((p) => `${p.key}=${p.lensId}`).join("|");
  const paramsKey = JSON.stringify(params);
  const refresh = useCallback(async () => {
    const current = guard.begin();
    const next = await Promise.all(
      parts.map(async ({ key, lensId }): Promise<[string, PartRun]> => {
        try {
          const lens = await getLens(lensId);
          return [key, { run: await runLens(lensId, childParams(lens, params), streamId), error: null }];
        } catch (e) {
          return [key, { run: null, error: e instanceof Error ? e.message : String(e) }];
        }
      }),
    );
    if (current()) setRuns(new Map(next));
    // partsKey / paramsKey stand in for `parts` / `params` (fresh each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [partsKey, paramsKey, streamId, guard]);
  useEffect(() => {
    guard.cancel();
    setRuns(null);
    void refresh();
  }, [refresh, guard]);
  useRerunOnChange(unionReads([...(runs?.values() ?? [])].map((r) => r.run?.result.reads)), () => void refresh());
  return runs;
}
