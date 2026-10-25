/// A window's open page runs (`.context/pages-and-tabs.md` "Page files"):
/// the daemon runs a page and re-runs it when a model one of its loaders
/// read changes; the window opens a run per page it shows, hears
/// `pageRunChanged` (which loaders' rows differ, never the rows) and
/// fetches the runs that changed, in one batch per burst. Nothing here
/// decides when a page re-runs: the daemon does.
///
/// Runs are shared by key (the page, its params, its stream and thread):
/// two hosts showing the same page open one run, and it closes when the
/// last lets go.
import { useEffect, useMemo, useState } from "react";

import { subscribeOxplowEvents, unwrapResult } from "./api.js";
import { getUiClientId } from "./logger.js";
import { commands, type PageRun, type SqlCell } from "./tauri-bridge/generated/bindings.js";
import { onRemoteReconnect } from "./tauri-bridge/transport.js";

/** What to run: a deployed page by id, or an inline page's YAML. */
export interface PageKey {
  page?: string;
  specYaml?: string;
  params: Record<string, SqlCell>;
  streamId: string | null;
  threadId: string | null;
}

/** What `pageRunChanged` says. */
export interface RunChange {
  runId: string;
  seq: number;
  loaders: string[];
  tree: boolean;
}

/** How the store reaches the daemon (injected, so tests need none). */
export interface PageRunsTransport {
  open(key: PageKey): Promise<{ runId: string; run: PageRun }>;
  close(runId: string): Promise<void>;
  fetch(runIds: string[]): Promise<Record<string, PageRun>>;
  setParams(runId: string, params: Record<string, SqlCell>): Promise<PageRun>;
  /** Run it again as it is (Refresh). */
  rerun(runId: string): Promise<PageRun>;
  /** `pageRunChanged` events; returns an unsubscribe. */
  onChange(handler: (change: RunChange) => void): () => void;
  /** The daemon came back (remote mode): every run is re-opened. */
  onReconnect(handler: () => void): () => void;
}

export interface RunState {
  /** The open run's id (what its actions are pressed on); null until the
   *  daemon has opened it. */
  runId: string | null;
  run: PageRun | null;
  error: string | null;
  /** The run shown is for the previous params; the new one is on its way. */
  stale: boolean;
}

export interface RunHandle {
  get(): RunState;
  subscribe(listener: () => void): () => void;
  /** New values for some params: the run re-runs with them. */
  setParams(params: Record<string, SqlCell>): Promise<void>;
  /** Run it again as it is (Refresh), the old run shown until it answers. */
  rerun(): Promise<void>;
  close(): void;
}

interface Entry {
  key: PageKey;
  runId: Promise<string | null>;
  state: RunState;
  listeners: Set<() => void>;
  refs: number;
  /** The newest change applied, so an older fetch is dropped. */
  seq: number;
  closeTimer: ReturnType<typeof setTimeout> | null;
}

const NO_RUN: RunState = { runId: null, run: null, error: null, stale: false };

/** How long changes gather before one fetch. */
export const COALESCE_MS = 50;
/** A host that drops a key and takes it straight back keeps its run. */
export const CLOSE_AFTER_MS = 250;

export function keyOf(key: PageKey): string {
  const params = Object.keys(key.params)
    .sort()
    .map((k) => [k, key.params[k]]);
  return JSON.stringify([key.page ?? null, key.specYaml ?? null, params, key.streamId, key.threadId]);
}

export function createPageRuns(transport: PageRunsTransport) {
  const entries = new Map<string, Entry>();
  const byRunId = new Map<string, Entry>();
  const pending = new Map<string, number>();
  let flushTimer: ReturnType<typeof setTimeout> | null = null;

  const notify = (entry: Entry) => {
    for (const l of entry.listeners) l();
  };

  const start = (entry: Entry) => {
    entry.runId = transport.open(entry.key).then(
      ({ runId, run }) => {
        byRunId.set(runId, entry);
        entry.state = { runId, run, error: null, stale: false };
        notify(entry);
        return runId;
      },
      (e: unknown) => {
        entry.state = { ...entry.state, error: String(e instanceof Error ? e.message : e), stale: false };
        notify(entry);
        return null;
      },
    );
  };

  const flush = async () => {
    flushTimer = null;
    const wanted = [...pending.entries()];
    pending.clear();
    if (wanted.length === 0) return;
    let runs: Record<string, PageRun>;
    try {
      runs = await transport.fetch(wanted.map(([id]) => id));
    } catch {
      return;
    }
    for (const [runId, seq] of wanted) {
      const entry = byRunId.get(runId);
      const run = runs[runId];
      if (!entry || !run || seq < entry.seq) continue;
      entry.seq = seq;
      entry.state = { runId, run, error: null, stale: false };
      notify(entry);
    }
  };

  // Stale at once: the host keeps the old run on screen, marked, until
  // the new one answers.
  const rerunWith = async (entry: Entry, rerun: (runId: string) => Promise<PageRun>) => {
    entry.state = { ...entry.state, stale: true };
    notify(entry);
    const runId = await entry.runId;
    if (!runId) {
      entry.state = { ...entry.state, stale: false };
      notify(entry);
      return;
    }
    try {
      const run = await rerun(runId);
      entry.state = { runId, run, error: null, stale: false };
    } catch (e) {
      entry.state = { ...entry.state, error: String(e instanceof Error ? e.message : e), stale: false };
    }
    notify(entry);
  };

  transport.onChange((change) => {
    const entry = byRunId.get(change.runId);
    if (!entry || change.seq <= entry.seq) return;
    pending.set(change.runId, Math.max(change.seq, pending.get(change.runId) ?? 0));
    if (flushTimer === null) flushTimer = setTimeout(() => void flush(), COALESCE_MS);
  });

  transport.onReconnect(() => {
    // The daemon may have restarted and forgotten every run: open each
    // again, keeping what is shown until the new run answers.
    byRunId.clear();
    for (const entry of entries.values()) {
      entry.seq = 0;
      start(entry);
    }
  });

  const open = (key: PageKey): RunHandle => {
    const k = keyOf(key);
    let entry = entries.get(k);
    if (entry) {
      if (entry.closeTimer) clearTimeout(entry.closeTimer);
      entry.closeTimer = null;
      entry.refs += 1;
    } else {
      entry = {
        key,
        runId: Promise.resolve(null),
        state: NO_RUN,
        listeners: new Set(),
        refs: 1,
        seq: 0,
        closeTimer: null,
      };
      entries.set(k, entry);
      start(entry);
    }
    const mine = entry;
    let closed = false;
    return {
      get: () => mine.state,
      subscribe: (listener) => {
        mine.listeners.add(listener);
        return () => mine.listeners.delete(listener);
      },
      setParams: (params) => rerunWith(mine, (runId) => transport.setParams(runId, params)),
      rerun: () => rerunWith(mine, (runId) => transport.rerun(runId)),
      close: () => {
        if (closed) return;
        closed = true;
        mine.refs -= 1;
        if (mine.refs > 0) return;
        mine.closeTimer = setTimeout(() => {
          if (mine.refs > 0) return;
          entries.delete(k);
          void mine.runId.then((runId) => {
            if (!runId) return;
            byRunId.delete(runId);
            void transport.close(runId).catch(() => {});
          });
        }, CLOSE_AFTER_MS);
      },
    };
  };

  return {
    open,
    /** The open runs' ids (the window's tab report names them, so the
     *  daemon drops a run no window holds). */
    runIds: () => [...byRunId.keys()],
  };
}

export type PageRuns = ReturnType<typeof createPageRuns>;

/** A host's run of `key` (none: no run), shared by key and kept fresh. */
export function usePageRunWith(runs: PageRuns, key: PageKey | null): RunState & {
  setParams: (params: Record<string, SqlCell>) => Promise<void>;
  rerun: () => Promise<void>;
} {
  const k = key ? keyOf(key) : null;
  // The key's text names it; `key` itself is a fresh object each render.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const handle = useMemo(() => (key ? runs.open(key) : null), [runs, k]);
  const [state, setState] = useState<RunState>(() => handle?.get() ?? NO_RUN);
  useEffect(() => {
    if (!handle) {
      setState(NO_RUN);
      return;
    }
    setState(handle.get());
    const off = handle.subscribe(() => setState(handle.get()));
    return () => {
      off();
      handle.close();
    };
  }, [handle]);
  return {
    ...state,
    setParams: (params) => handle?.setParams(params) ?? Promise.resolve(),
    rerun: () => handle?.rerun() ?? Promise.resolve(),
  };
}

/** The daemon, as `createPageRuns` reaches it. */
export function daemonTransport(): PageRunsTransport {
  return {
    open: async (key) =>
      unwrapResult(
        await commands.openPage(
          key.page ?? null,
          key.specYaml ?? null,
          key.params,
          key.streamId,
          key.threadId,
          getUiClientId(),
        ),
      ),
    close: async (runId) => {
      unwrapResult(await commands.closePage(runId));
    },
    fetch: async (runIds) => unwrapResult(await commands.fetchPageRuns(runIds)) as Record<string, PageRun>,
    setParams: async (runId, params) => unwrapResult(await commands.setPageParams(runId, params)),
    rerun: async (runId) => unwrapResult(await commands.rerunPage(runId)),
    onChange: (handler) =>
      subscribeOxplowEvents((event) => {
        if (event.kind === "pageRunChanged") {
          handler({ runId: event.runId, seq: Number(event.seq), loaders: event.loaders, tree: event.tree });
        }
      }),
    onReconnect: (handler) => onRemoteReconnect(handler),
  };
}

let shared: PageRuns | null = null;

/** The window's page runs, over the daemon. */
export function windowPageRuns(): PageRuns {
  shared ??= createPageRuns(daemonTransport());
  return shared;
}

/** A host's run of `key` in this window (none: no run). */
export function usePageRun(key: PageKey | null) {
  return usePageRunWith(windowPageRuns(), key);
}
