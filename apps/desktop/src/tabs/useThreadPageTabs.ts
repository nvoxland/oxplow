import { useEffect, useState } from "react";

import type { TabRef } from "./tabState.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import {
  readPersistedDiffSpecs,
  readPersistedThreadPageHistory,
  readPersistedThreadPageTabs,
  writePersistedDiffSpecs,
  writePersistedThreadPageHistory,
  writePersistedThreadPageTabs,
  type ThreadHistory,
} from "./pageTabsPersistence.js";

/**
 * Owns the per-thread page-tab layout state and its localStorage
 * persistence:
 *
 * - `threadPageTabs` — per-thread open page tabs (TabRef[]); the
 *   unified source of truth for tab order across page kinds.
 * - `threadPageHistory` — per-tab browser-style back/forward stacks.
 * - `diffTabs` — the diff-spec registry, indexed by tab id (diff tabs
 *   themselves live in `threadPageTabs`; this carries their specs).
 * - `threadCenterActive` — per-thread last-active center tab pointer
 *   (in-memory; the cross-restart seed is the legacy global
 *   centerActive key, still handled by the shell).
 *
 * State restores from localStorage on mount and writes back on every
 * change. App.tsx consumes this as the single owner of tab layout.
 */
export function useThreadPageTabs() {
  const [threadCenterActive, setThreadCenterActive] = useState<Record<string, string>>({});
  // Per-thread MRU (most-recently-used first) order of tab ids, used to
  // pick LRU eviction victims when a thread exceeds the page-tab cap.
  // In-memory only (like `threadCenterActive`): recency is a session
  // concern, rebuilt as tabs are activated.
  const [threadPageMru, setThreadPageMru] = useState<Record<string, string[]>>({});
  const [threadPageTabs, setThreadPageTabs] = useState<Record<string, TabRef[]>>(() =>
    readPersistedThreadPageTabs(),
  );
  const [threadPageHistory, setThreadPageHistory] = useState<ThreadHistory>(() =>
    readPersistedThreadPageHistory(),
  );
  const [diffTabs, setDiffTabs] = useState<Array<{ id: string; spec: DiffSpec }>>(() =>
    readPersistedDiffSpecs(),
  );

  // Persist the per-thread tab list + per-tab history. Pages mount
  // fresh on the next boot — the snapshot layer (usePageSnapshot)
  // rehydrates per-page state separately.
  useEffect(() => {
    writePersistedThreadPageTabs(threadPageTabs);
  }, [threadPageTabs]);
  useEffect(() => {
    writePersistedThreadPageHistory(threadPageHistory);
  }, [threadPageHistory]);
  useEffect(() => {
    writePersistedDiffSpecs(diffTabs);
  }, [diffTabs]);

  return {
    threadCenterActive,
    setThreadCenterActive,
    threadPageMru,
    setThreadPageMru,
    threadPageTabs,
    setThreadPageTabs,
    threadPageHistory,
    setThreadPageHistory,
    diffTabs,
    setDiffTabs,
  };
}
