/// The one owner of every extension panel's runs (page runs, or one open
/// page run per part for a page-backed panel), shared by
/// the rail (each panel's body, count and summary), the status bar's bell
/// and the Alerts page (the badges that fire): each page runs once per
/// refresh, and the three can't disagree. Provided by the app around
/// everything that reads it.

import type { ReactNode } from "react";
import { createContext, useCallback, useContext, useMemo } from "react";

import { panelScope, useRememberedStore } from "../../prefs/remembered.js";

import { useExtensions } from "../../extensionsStore.js";
import type { PageRuns } from "../../pageRuns.js";
import type { ExtensionPanel } from "../../tauri-bridge/generated/bindings.js";
import {
  NO_CHOICES,
  panelAlerts,
  useExtensionPanelRuns,
  type PanelAlert,
  type PanelChoices,
  type PanelRuns,
} from "./usePanelRuns.js";

export interface PanelRunsValue {
  /** The enabled extensions' panels for the stream. */
  panels: ExtensionPanel[];
  /** Each panel's runs, by panel id. */
  runs: Record<string, PanelRuns>;
  /** The badges that fire. */
  alerts: PanelAlert[];
  /** The viewer's picks for the panels' choice params. */
  choices: PanelChoices;
  /** Pick `value` for a panel's choice param; its body re-runs. */
  choose(panelId: string, param: string, value: string): void;
}

const EMPTY: PanelRunsValue = { panels: [], runs: {}, alerts: [], choices: NO_CHOICES, choose: () => {} };

/** Each panel's picks are its memory, kept under this key. */
const CHOICES_KEY = "choices";

const PanelRunsContext = createContext<PanelRunsValue>(EMPTY);

export function PanelRunsProvider({
  streamId,
  threadId,
  pageRuns,
  children,
}: {
  streamId: string | null;
  threadId: string | null;
  /** The page runs page-backed panels open (the window's; a test's fake). */
  pageRuns?: PageRuns;
  children: ReactNode;
}) {
  const exts = useExtensions();
  const panels = useMemo(() => (exts ?? []).filter((e) => e.enabled).flatMap((e) => e.panels), [exts]);
  const store = useRememberedStore();
  const version = store.version();
  const choices = useMemo(() => {
    const out: PanelChoices = {};
    for (const p of panels) {
      const kept = store.get(panelScope(p.id), CHOICES_KEY);
      if (kept && typeof kept === "object") out[p.id] = kept as Record<string, string>;
    }
    return Object.keys(out).length === 0 ? NO_CHOICES : out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, panels, version]);
  const choose = useCallback(
    (panelId: string, param: string, value: string) => {
      const kept = store.get(panelScope(panelId), CHOICES_KEY);
      store.set(panelScope(panelId), CHOICES_KEY, { ...(kept && typeof kept === "object" ? kept : {}), [param]: value });
    },
    [store],
  );
  const runs = useExtensionPanelRuns(panels, streamId, threadId, choices, pageRuns);
  const value = useMemo(
    () => ({ panels, runs, alerts: panelAlerts(panels, runs, exts ?? []), choices, choose }),
    [panels, runs, choices, choose, exts],
  );
  return <PanelRunsContext.Provider value={value}>{children}</PanelRunsContext.Provider>;
}

/** The shared panel runs (none outside a provider). */
export function usePanelRuns(): PanelRunsValue {
  return useContext(PanelRunsContext);
}
