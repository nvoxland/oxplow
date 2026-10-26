/// The dock's panels: what each icon in the bar opens. Core registers its
/// own (Alerts now; the assistant and the context basket join through
/// `registerDockPanel`); an extension's `panels:` entry with `dock: true`
/// joins as a panel whose body is its page. `useDockPanels` is the one list
/// the bar and the panel read, in a fixed order: core's first (by
/// `CORE_ORDER`), then the extensions' by title.

import type { ComponentType } from "react";
import { useMemo } from "react";

import type { ExtensionPanel } from "../../tauri-bridge/generated/bindings.js";
import type { TabRef } from "../../tabs/tabState.js";
import { AssistantDockBody, useAssistantBadge } from "../Assistant/AssistantDockPanel.js";
import { ContextDockBody, useContextBadge } from "../Context/ContextDockPanel.js";
import { AlertsDockBody, useAlertsBadge } from "../Alerts/AlertsDockPanel.js";
import { usePanelRuns } from "../Panels/PanelRunsContext.js";
import { badgeCount } from "../Panels/usePanelRuns.js";
import { PageTree } from "../../pages/PageTree.js";

export interface DockBodyProps {
  /** Open a page (or focus it) in the active thread's tab area. */
  onOpenPage(ref: TabRef): void;
  streamId: string | null;
}

export interface DockBadge {
  count: number;
  /** `danger` paints the pill red; the default is the accent. */
  tone?: "danger";
}

export interface DockPanelDef {
  id: string;
  title: string;
  /** A name from `ICONS`. */
  icon: string;
  /** A hook: the badge for the icon, or null for none. */
  useBadge(): DockBadge | null;
  /** The body; mounted only while the panel is open. */
  Body: ComponentType<DockBodyProps>;
}

/** Where a core panel sits in the bar. */
export const CORE_ORDER = ["assistant", "context", "alerts"];

const core: DockPanelDef[] = [
  { id: "assistant", title: "Assistant", icon: "sparkles", useBadge: useAssistantBadge, Body: AssistantDockBody },
  { id: "context", title: "Context", icon: "at-sign", useBadge: useContextBadge, Body: ContextDockBody },
  { id: "alerts", title: "Alerts", icon: "bell", useBadge: useAlertsBadge, Body: AlertsDockBody },
];

/** Add (or replace) a core panel. */
export function registerDockPanel(def: DockPanelDef): void {
  const at = core.findIndex((p) => p.id === def.id);
  if (at >= 0) core[at] = def;
  else core.push(def);
}

/** Core panels in `CORE_ORDER`, then `extensions` by title. */
export function orderDockPanels(corePanels: DockPanelDef[], extensions: DockPanelDef[]): DockPanelDef[] {
  const rank = (p: DockPanelDef) => {
    const i = CORE_ORDER.indexOf(p.id);
    return i < 0 ? CORE_ORDER.length : i;
  };
  return [
    ...[...corePanels].sort((a, b) => rank(a) - rank(b)),
    ...[...extensions].sort((a, b) => a.title.localeCompare(b.title)),
  ];
}

/** An extension panel as a dock panel: the body page, and the count as the badge. */
export function extensionDockPanel(panel: ExtensionPanel): DockPanelDef {
  return {
    id: `ext:${panel.id}`,
    title: panel.title,
    icon: panel.icon ?? "box",
    useBadge() {
      const runs = usePanelRuns().runs[panel.id];
      const count = runs?.count ?? badgeCount(runs?.badge ?? null);
      return count && count > 0 ? { count } : null;
    },
    Body({ onOpenPage, streamId }) {
      const runs = usePanelRuns().runs[panel.id];
      const page = runs?.body;
      return page ? (
        <div style={{ padding: "8px 12px", fontSize: "var(--text-xs)" }}>
          <PageTree run={page.run} runId={page.runId} streamId={streamId} onOpenPage={onOpenPage} />
        </div>
      ) : (
        <div style={{ padding: "8px 12px", color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Loading…</div>
      );
    },
  };
}

export function useDockPanels(): DockPanelDef[] {
  const { panels } = usePanelRuns();
  return useMemo(
    () => orderDockPanels(core, panels.filter((p) => p.dock).map(extensionDockPanel)),
    [panels],
  );
}
