/// The one owner of every extension panel's lens runs, shared by
/// the rail (each panel's body, count and summary), the status bar's bell
/// and the Alerts page (the badges that fire): each lens runs once per
/// refresh, and the three can't disagree. Provided by the app around
/// everything that reads it.

import type { ReactNode } from "react";
import { createContext, useCallback, useContext, useMemo, useState } from "react";

import { useExtensions } from "../../extensionsStore.js";
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

// The picks are a viewer convenience, remembered in this browser only.
const CHOICES_KEY = "oxplow.panelChoices.v1";

function loadChoices(): PanelChoices {
  try {
    const raw = localStorage.getItem(CHOICES_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === "object" ? (parsed as PanelChoices) : NO_CHOICES;
  } catch {
    return NO_CHOICES;
  }
}

function saveChoices(choices: PanelChoices): void {
  try {
    localStorage.setItem(CHOICES_KEY, JSON.stringify(choices));
  } catch {
    // Unavailable storage: the pick lasts this session.
  }
}
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
  const [choices, setChoices] = useState<PanelChoices>(loadChoices);
  const choose = useCallback((panelId: string, param: string, value: string) => {
    setChoices((prev) => {
      const next = { ...prev, [panelId]: { ...(prev[panelId] ?? {}), [param]: value } };
      saveChoices(next);
      return next;
    });
  }, []);
  const runs = useExtensionPanelRuns(panels, streamId, threadId, choices);
  const value = useMemo(
    () => ({ panels, runs, alerts: panelAlerts(panels, runs), choices, choose }),
    [panels, runs, choices, choose],
  );
  return <PanelRunsContext.Provider value={value}>{children}</PanelRunsContext.Provider>;
}

/** The shared panel runs (none outside a provider). */
export function usePanelRuns(): PanelRunsValue {
  return useContext(PanelRunsContext);
}
