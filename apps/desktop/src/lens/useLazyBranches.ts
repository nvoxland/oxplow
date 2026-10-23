import { useEffect, useRef, useState } from "react";

import { runLens, type LensRun, type SqlCell } from "../api.js";
import { lazyTreeProblem, limitRows, wantedBranches, type BranchRun } from "./lensModel.js";
import { unionReads, useRerunOnChange } from "./lensRerun.js";

/**
 * A lazy tree's branch runs (`tree.lazy`): for each open branch reachable
 * through open parents (`wantedBranches`), the lens run again with its
 * lazy param set to the branch's id, in `streamId`, keyed by that id.
 * Opening one branch leaves the others' rows in place; closing one drops
 * its run; a late answer for a branch that ran again is dropped (latest
 * wins per branch). The loaded branches run again when what they read
 * changes. `maxRows` caps each branch's rows as the root's are capped.
 */
export function useLazyBranches(run: LensRun, open: readonly string[], streamId: string | null, maxRows?: number): ReadonlyMap<string, BranchRun> {
  const [runs, setRuns] = useState<ReadonlyMap<string, BranchRun>>(() => new Map());
  // The request in flight per branch: an answer lands only while its
  // version is current.
  const versions = useRef(new Map<string, number>());
  const lazy = run.lens.tree?.lazy ?? null;
  const problem = lazyTreeProblem(run.lens, run.result);
  const wanted = lazy && !problem ? wantedBranches(run.lens, run.result, new Set(open), runs) : [];
  const wantedKey = wanted.map((w) => w.id).join("\u0000");
  const fetch = (branches: { id: string; value: SqlCell }[]) => {
    for (const { id, value } of branches) {
      const version = (versions.current.get(id) ?? 0) + 1;
      versions.current.set(id, version);
      const current = () => versions.current.get(id) === version;
      void runLens(run.lens.id, { ...run.params, [lazy!]: value }, streamId)
        .then((r) => ({ run: limitRows(r.result, maxRows) === r.result ? r : { ...r, result: limitRows(r.result, maxRows) }, error: null }) as BranchRun)
        .catch((e: unknown) => ({ run: null, error: e instanceof Error ? e.message : String(e) }) as BranchRun)
        .then((got) => {
          if (current()) setRuns((prev) => new Map(prev).set(id, got));
        });
    }
  };
  useEffect(() => {
    // Branches no longer wanted go; the ones still missing run.
    const keep = new Set(wanted.map((w) => w.id));
    setRuns((prev) => ([...prev.keys()].every((k) => keep.has(k)) ? prev : new Map([...prev].filter(([k]) => keep.has(k)))));
    for (const k of [...versions.current.keys()]) if (!keep.has(k)) versions.current.delete(k);
    fetch(wanted.filter((w) => !runs.has(w.id)));
    // wantedKey stands in for `wanted`; `runs` is read, not depended on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wantedKey, run.lens.id, streamId]);
  useEffect(() => () => versions.current.clear(), []);
  const loaded = [...runs.values()].map((r) => r.run?.result.reads);
  useRerunOnChange(unionReads(loaded), () => fetch(wanted.filter((w) => runs.has(w.id))));
  return runs;
}
