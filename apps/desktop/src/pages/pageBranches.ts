/// A page node's lazy tree (`tree.lazy`, `.context/pages-and-tabs.md` "The
/// desktop host"): each open branch is a run of the node's page of its
/// own, the lazy param set to the branch's id — opened like any page run
/// (shared by key, kept fresh by the daemon) and dropped when the branch
/// closes. The node's loader in it is the branch's rows; its rows press on
/// that run, which holds them.
import { useEffect, useMemo, useRef, useState } from "react";

import { limitRows, wantedBranches, type BranchRun } from "../lens/lensModel.js";
import { EMPTY_RESULT } from "../lens/nodeRun.js";
import { keyOf, type PageKey, type PageRuns, type RunHandle, type RunState } from "../pageRuns.js";
import type { LensRun, Node, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { nodeActionRunner, nodeFormSource, type PageForms, type RunPageAction } from "./useNodeActions.js";

/** One run per key of `keys` (by id), opened and closed as the keys
 *  come and go; their states by id. */
export function usePageRunsFor(store: PageRuns, keys: ReadonlyMap<string, PageKey>): ReadonlyMap<string, RunState> {
  const held = useRef(new Map<string, { key: string; handle: RunHandle; off(): void }>());
  const [states, setStates] = useState<ReadonlyMap<string, RunState>>(() => new Map());
  const keysKey = JSON.stringify([...keys].map(([id, k]) => [id, keyOf(k)]));
  useEffect(() => {
    const cur = held.current;
    const sync = () => setStates(new Map([...cur].map(([id, h]) => [id, h.handle.get()])));
    for (const [id, h] of [...cur]) {
      const want = keys.get(id);
      if (want && keyOf(want) === h.key) continue;
      h.off();
      h.handle.close();
      cur.delete(id);
    }
    for (const [id, key] of keys) {
      if (cur.has(id)) continue;
      const handle = store.open(key);
      cur.set(id, { key: keyOf(key), handle, off: handle.subscribe(sync) });
    }
    sync();
    // `keysKey` is `keys`' content.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, keysKey]);
  useEffect(
    () => () => {
      for (const h of held.current.values()) {
        h.off();
        h.handle.close();
      }
      held.current.clear();
    },
    [],
  );
  return states;
}

/** What the branches of viz node `node` in `pageRun` read, and where
 *  their rows press. */
export interface BranchHost {
  pageRun: PageRun;
  node: Node;
  store: PageRuns;
  press: RunPageAction;
  forms: PageForms;
}

/** The lazy branch runs of a page's tree node: `useLazyBranches`'s shape
 *  (`VizPages.useBranches`), its runs pages'. */
export function usePageBranches(host: BranchHost, run: LensRun, open: readonly string[], streamId: string | null, maxRows?: number): ReadonlyMap<string, BranchRun> {
  const { pageRun, node, store, press, forms } = host;
  const lazy = run.lens.tree?.lazy ?? null;
  const from = node.kind === "viz" ? node.from : null;
  const [loaded, setLoaded] = useState<ReadonlyMap<string, BranchRun>>(() => new Map());
  const wanted = lazy ? wantedBranches(run.lens, run.result, new Set(open), loaded) : [];
  const keys = useMemo(
    () => new Map(wanted.map((w) => [w.id, { page: pageRun.page.id, params: { ...pageRun.params, [lazy!]: w.value as SqlCell }, streamId, threadId: null } as PageKey])),
    // The wanted ids and values name the keys.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [JSON.stringify(wanted), pageRun.page.id, JSON.stringify(pageRun.params), lazy, streamId],
  );
  const states = usePageRunsFor(store, keys);
  const branches = useMemo(() => {
    const out = new Map<string, BranchRun>();
    for (const [id, state] of states) {
      if (!state.run) {
        if (state.error) out.set(id, { run: null, error: state.error });
        continue;
      }
      const got = from !== null ? state.run.loaders[from] : undefined;
      if (got?.error) {
        out.set(id, { run: null, error: got.error });
        continue;
      }
      out.set(id, {
        run: { ...run, params: state.run.params, result: limitRows(got?.result ?? EMPTY_RESULT, maxRows), alert: null, warnings: [], inactive: null },
        error: null,
        runner: nodeActionRunner(press, state.runId, node.id),
        forms: nodeFormSource(forms, state.runId, node.id),
      });
    }
    return out;
    // `run.lens` is what the branch rows draw as; its rows are the root's.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [states, from, run.lens, maxRows, press, forms, node.id]);
  // What the next render's wanted branches are reached through.
  useEffect(() => setLoaded(branches), [branches]);
  return branches;
}
