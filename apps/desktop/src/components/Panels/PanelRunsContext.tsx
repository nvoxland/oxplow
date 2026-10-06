/// The one owner of every extension panel's lens runs (tsk1097), shared by
/// the rail (each panel's body, count and summary), the status bar's bell
/// and the Alerts page (the badges that fire): each lens runs once per
/// refresh, and the three can't disagree. Provided by the app around
/// everything that reads it.

import type { ReactNode } from "react";
import { createContext, useContext, useMemo } from "react";

import { useExtensions } from "../../extensionsStore.js";
import type { ExtensionPanel } from "../../tauri-bridge/generated/bindings.js";
import { panelAlerts, useExtensionPanelRuns, type PanelAlert, type PanelRuns } from "./usePanelRuns.js";

export interface PanelRunsValue {
  /** The enabled extensions' panels for the stream. */
  panels: ExtensionPanel[];
  /** Each panel's runs, by panel id. */
  runs: Record<string, PanelRuns>;
  /** The badges that fire. */
  alerts: PanelAlert[];
}

const EMPTY: PanelRunsValue = { panels: [], runs: {}, alerts: [] };
const PanelRunsContext = createContext<PanelRunsValue>(EMPTY);

export function PanelRunsProvider({
  streamId,
  threadId,
  children,
}: {
  streamId: string | null;
  threadId: string | null;
  children: ReactNode;
}) {
  const exts = useExtensions(streamId);
  const panels = useMemo(() => (exts ?? []).filter((e) => e.enabled).flatMap((e) => e.panels), [exts]);
  const runs = useExtensionPanelRuns(panels, streamId, threadId);
  const value = useMemo(() => ({ panels, runs, alerts: panelAlerts(panels, runs) }), [panels, runs]);
  return <PanelRunsContext.Provider value={value}>{children}</PanelRunsContext.Provider>;
}

/** The shared panel runs (none outside a provider). */
export function usePanelRuns(): PanelRunsValue {
  return useContext(PanelRunsContext);
}
