/// A window's open page runs (`.context/pages-and-tabs.md` "Page files"):
/// the daemon runs a page and re-runs it when a dataset one of its loaders
/// read changes; the window opens a run per page it shows, hears
/// `pageRunChanged` (which loaders' rows differ, never the rows) and
/// fetches the runs that changed, in one batch per burst. Nothing here
/// decides when a page re-runs: the daemon does.
///
/// Runs are shared by key (the page, its params, its stream and thread):
/// two hosts showing the same page open one run, and it closes when the
/// last lets go.
import { createContext, useContext, useEffect, useMemo, useState } from "react";

import { subscribeOxplowEvents, unwrapResult } from "./api.js";
import { getUiClientId } from "./uiClientId.js";
import { commands, type PageRun, type RunSnapshot, type SqlCell } from "./tauri-bridge/generated/bindings.js";
import { onRemoteReconnect } from "./tauri-bridge/transport.js";

/** What to run: a page by id (a deployed one, or a temp view,
 *  `temp/<id>`), or an inline page's YAML (a markdown block's). */
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
  fetch(runIds: string[]): Promise<Record<string, RunSnapshot>>;
  setParams(runId: string, params: Record<string, SqlCell>): Promise<PageRun>;
  /** Run it again as it is (Refresh). */
  rerun(runId: string): Promise<PageRun>;
  /** Whether the run's tab is on screen. The daemon does not re-run a
   *  hidden run; shown again after a missed change, it runs once (true:
   *  fetch it). */
  setVisible(runId: string, visible: boolean): Promise<boolean>;
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
  /** This host's tab is on screen (the default) or not. */
  setVisible(visible: boolean): void;
  close(): void;
}

interface Entry {
  /** Fences an old open reply after reconnect or final release. */
  opening: number;
  operation: number;
  /** Current desired params; key remains the shared subscription identity. */
  requestParams: Record<string, SqlCell>;
  key: PageKey;
  runId: Promise<string | null>;
  state: RunState;
  listeners: Set<() => void>;
  refs: number;
  /** Hosts whose tab is on screen; the run is paused at none. */
  shown: number;
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
  const rosterListeners = new Set<() => void>();
  const rosterChanged = () => { for (const listener of rosterListeners) listener(); };
  const pending = new Map<string, number>();
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  let inFlightOpens = 0;
  const earlyChanges = new Map<string, number>();
  let earlyOverflow = false;

  const scheduleFetch = (runId: string, seq: number) => {
    pending.set(runId, Math.max(seq, pending.get(runId) ?? 0));
    if (flushTimer === null) flushTimer = setTimeout(() => void flush(), COALESCE_MS);
  };

  const notify = (entry: Entry) => {
    for (const l of entry.listeners) l();
  };

  const start = (entry: Entry) => {
    const opening = ++entry.opening;
    inFlightOpens += 1;
    entry.runId = transport.open({ ...entry.key, params: { ...entry.requestParams } }).then(
      ({ runId, run }) => {
        const earlySeq = earlyChanges.get(runId);
        earlyChanges.delete(runId);
        if (opening !== entry.opening) {
          void transport.close(runId).catch(() => {});
          return null;
        }
        byRunId.set(runId, entry);
        entry.state = { runId, run, error: null, stale: false };
        // The daemon may have announced a change before its open HTTP
        // reply reached this window. Keep notices only while opens are
        // pending, never as a timed history of unrelated events.
        if (earlySeq !== undefined || earlyOverflow) scheduleFetch(runId, earlySeq ?? 0);
        rosterChanged();
        notify(entry);
        return runId;
      },
      (e: unknown) => {
        if (opening !== entry.opening) return null;
        entry.state = { ...entry.state, error: String(e instanceof Error ? e.message : e), stale: false };
        notify(entry);
        return null;
      },
    ).finally(() => {
      inFlightOpens -= 1;
      if (inFlightOpens === 0) {
        earlyChanges.clear();
        earlyOverflow = false;
      }
    });
  };

  const flush = async () => {
    flushTimer = null;
    const wanted = [...pending.entries()];
    pending.clear();
    if (wanted.length === 0) return;
    const operations = new Map(wanted.map(([id]) => [id, byRunId.get(id)?.operation]));
    let runs: Record<string, RunSnapshot>;
    try {
      runs = await transport.fetch(wanted.map(([id]) => id));
    } catch {
      return;
    }
    for (const [runId] of wanted) {
      const entry = byRunId.get(runId);
      const snapshot = runs[runId];
      if (!entry || !snapshot || snapshot.seq < entry.seq) continue;
      if (entry.operation !== operations.get(runId)) {
        scheduleFetch(runId, 0);
        continue;
      }
      entry.seq = snapshot.seq;
      entry.state = { ...entry.state, runId, run: snapshot.run, error: null, stale: false };
      notify(entry);
    }
  };

  // Stale at once: the host keeps the old run on screen, marked, until
  // the new one answers.
  const rerunWith = async (entry: Entry, rerun: (runId: string) => Promise<PageRun>, params?: Record<string, SqlCell>) => {
    const opening = entry.opening;
    const operation = ++entry.operation;
    const initialSeq = entry.seq;
    const previousParams = entry.requestParams;
    if (params) entry.requestParams = { ...entry.requestParams, ...params };
    const current = () => opening === entry.opening && operation === entry.operation;
    entry.state = { ...entry.state, stale: true };
    notify(entry);
    const runId = await entry.runId;
    if (!current()) return;
    if (!runId) {
      entry.state = { ...entry.state, stale: false };
      ++entry.operation;
      notify(entry);
      return;
    }
    try {
      const run = await rerun(runId);
      if (!current() || entry.state.runId !== runId) return;
      if (entry.seq !== initialSeq) {
        entry.state = { ...entry.state, stale: false };
        scheduleFetch(runId, 0);
      } else entry.state = { ...entry.state, runId, run, error: null, stale: false };
    } catch (e) {
      if (!current() || entry.state.runId !== runId) return;
      if (params) entry.requestParams = previousParams;
      if (entry.seq !== initialSeq) {
        entry.state = { ...entry.state, stale: false };
        scheduleFetch(runId, 0);
      } else entry.state = { ...entry.state, error: String(e instanceof Error ? e.message : e), stale: false };
    }
    // A fetch begun while the operation was running may have captured its
    // precommit state too, so completion advances the fetch fence again.
    ++entry.operation;
    notify(entry);
  };

  transport.onChange((change) => {
    const entry = byRunId.get(change.runId);
    if (!entry) {
      if (inFlightOpens > 0) {
        if (earlyChanges.has(change.runId) || earlyChanges.size < 1024) {
          earlyChanges.set(change.runId, Math.max(change.seq, earlyChanges.get(change.runId) ?? 0));
        } else earlyOverflow = true;
      }
      return;
    }
    if (change.seq <= entry.seq) return;
    scheduleFetch(change.runId, change.seq);
  });

  /** Open `entry` again, keeping what is shown until the new run answers. */
  const reopenEntry = (entry: Entry) => {
    entry.seq = 0;
    entry.state = { ...entry.state, runId: null, stale: true };
    notify(entry);
    start(entry);
  };

  transport.onReconnect(() => {
    // The daemon may have restarted and forgotten every run.
    byRunId.clear();
    rosterChanged();
    for (const entry of entries.values()) reopenEntry(entry);
  });

  const open = (key: PageKey): RunHandle => {
    const k = keyOf(key);
    let entry = entries.get(k);
    // A host that opens onto a run whose hosts were all hidden shows it
    // again: the daemon (which stopped re-running it) is told below.
    let wakes = false;
    if (entry) {
      if (entry.closeTimer) clearTimeout(entry.closeTimer);
      entry.closeTimer = null;
      entry.refs += 1;
      wakes = entry.shown === 0;
      entry.shown += 1;
    } else {
      entry = {
        opening: 0,
        operation: 0,
        requestParams: { ...key.params },
        key,
        runId: Promise.resolve(null),
        state: NO_RUN,
        listeners: new Set(),
        refs: 1,
        shown: 1,
        seq: 0,
        closeTimer: null,
      };
      entries.set(k, entry);
      start(entry);
    }
    const mine = entry;
    let closed = false;
    let shown = true;
    const tell = (visible: boolean) => {
      void mine.runId.then(async (runId) => {
        if (!runId || mine.shown > 0 !== visible) return;
        try {
          if (await transport.setVisible(runId, visible)) scheduleFetch(runId, 0);
        } catch {
          // The run is gone (closed, or the daemon restarted): reopening
          // runs it fresh.
        }
      });
    };
    if (wakes) tell(true);
    return {
      setVisible: (visible) => {
        if (closed || visible === shown) return;
        shown = visible;
        mine.shown += visible ? 1 : -1;
        if (visible ? mine.shown === 1 : mine.shown === 0) tell(visible);
      },
      get: () => mine.state,
      subscribe: (listener) => {
        mine.listeners.add(listener);
        return () => mine.listeners.delete(listener);
      },
      setParams: (params) => rerunWith(mine, (runId) => transport.setParams(runId, params), params),
      rerun: () => rerunWith(mine, (runId) => transport.rerun(runId)),
      close: () => {
        if (closed) return;
        closed = true;
        mine.refs -= 1;
        if (shown) mine.shown -= 1;
        if (mine.refs > 0) return;
        mine.closeTimer = setTimeout(() => {
          if (mine.refs > 0) return;
          entries.delete(k);
          ++mine.opening;
          if (mine.state.runId && byRunId.delete(mine.state.runId)) rosterChanged();
          void mine.runId.then((runId) => {
            if (!runId) return;
            void transport.close(runId).catch(() => {});
          });
        }, CLOSE_AFTER_MS);
      },
    };
  };

  return {
    open,
    /** Run open run `runId` again as it is, for every host holding it:
     *  after the person's press on it, whose command may have changed
     *  what no dataset change announces (a keychain entry). Unknown: nothing. */
    refresh: async (runId: string): Promise<void> => {
      const entry = byRunId.get(runId);
      if (entry) await rerunWith(entry, (id) => transport.rerun(id));
    },
    /** The open runs' ids (the window's tab report names them, so the
     *  daemon drops a run no window holds). */
    runIds: () => [...byRunId.keys()],
    /** The daemon says it no longer holds these (it forgot a window that went
     *  quiet): open them again, or they would show their last rows for good.
     *  An id this window has since let go of is nothing to reopen. */
    reopen: (runIds: readonly string[]) => {
      for (const runId of runIds) {
        const entry = byRunId.get(runId);
        if (!entry) continue;
        byRunId.delete(runId);
        reopenEntry(entry);
      }
      if (runIds.length > 0) rosterChanged();
    },
    /** Add/remove/reconnect notices, including panels and markdown runs. */
    subscribeRunIds: (listener: () => void) => {
      rosterListeners.add(listener);
      return () => { rosterListeners.delete(listener); };
    },
  };
}

export type PageRuns = ReturnType<typeof createPageRuns>;

/** A host's run of `key` (none: no run), shared by key and kept fresh. */
export function usePageRunWith(runs: PageRuns, key: PageKey | null, visible = true): RunState & {
  setParams: (params: Record<string, SqlCell>) => Promise<void>;
  rerun: () => Promise<void>;
} {
  const k = key ? keyOf(key) : null;
  // The key's text names it; `key` itself is a fresh object each render.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const handle = useMemo(() => (key ? runs.open(key) : null), [runs, k]);
  const [state, setState] = useState<RunState>(() => handle?.get() ?? NO_RUN);
  useEffect(() => {
    handle?.setVisible(visible);
  }, [handle, visible]);
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
    fetch: async (runIds) => unwrapResult(await commands.fetchPageRuns(runIds)),
    setParams: async (runId, params) => unwrapResult(await commands.setPageParams(runId, params)),
    rerun: async (runId) => unwrapResult(await commands.rerunPage(runId)),
    setVisible: async (runId, visible) => unwrapResult(await commands.setPageVisible(runId, visible)),
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

/** The page runs below it (a test's fake); none: the window's. */
export const PageRunsContext = createContext<PageRuns | null>(null);

/** The page runs in force: the context's, else the window's. */
export function usePageRunsStore(): PageRuns {
  return useContext(PageRunsContext) ?? windowPageRuns();
}

/** A host's run of `key` in this window (none: no run). */
export function usePageRun(key: PageKey | null) {
  return usePageRunWith(windowPageRuns(), key);
}
