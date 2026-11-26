/// The dock's panels: what each icon in the bar opens. Core registers its
/// own (Alerts now; the context basket joins through
/// `registerDockPanel`); an extension's `panels:` entry with `dock: true`
/// joins as a panel whose body is its page. `useDockPanels` is the one list
/// the bar and the panel read, in a fixed order: core's first (by
/// `CORE_ORDER`), then the extensions' by title.

import type { ComponentType } from "react";
import { useMemo } from "react";

import type { Extension, ExtensionPanel } from "../../tauri-bridge/generated/bindings.js";
import type { TabRef } from "../../tabs/tabState.js";
import { alertsRef, contextRef } from "../../tabs/pageRefs.js";
import { useExtensions } from "../../extensionsStore.js";
import { panelOpenRef } from "../Panels/panelLayout.js";
import { ContextDockBody, useContextBadge } from "../Context/ContextDockPanel.js";
import { AlertsDockBody, useAlertsBadge } from "../Alerts/AlertsDockPanel.js";
import { usePanelRuns } from "../Panels/PanelRunsContext.js";
import { PanelBody, PanelChoiceToggle, PanelCollapsed } from "../Panels/PanelParts.js";

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
  /** Controls for the open panel's header (an extension panel's choice
   *  toggles). Mounted only while the panel is open. */
  Header?: ComponentType<DockBodyProps>;
  /** The panel's collapsed form: a summary the bar draws beside the icon
   *  while the panel is closed. It renders nothing when it has none. */
  Collapsed?: ComponentType<DockBodyProps>;
  /** The page the panel's things are on (the Alerts page, the Context
   *  page, an extension panel's `open` or body page): the open arrow in
   *  the panel's header, as a section heading's (`OpenPageLink`). */
  page?: TabRef;
}

/** Where a core panel sits in the bar. A closed list: it only shrinks
 *  (`dockPanels.test.ts` pins it against `corePanels.golden.json`) — a panel
 *  that isn't core's own is an extension's `panels:` entry with `dock: true`. */
export const CORE_ORDER = ["alerts", "context"];

const core: DockPanelDef[] = [
  { id: "context", title: "Context", icon: "at-sign", useBadge: useContextBadge, Body: ContextDockBody, page: contextRef() },
  { id: "alerts", title: "Alerts", icon: "bell", useBadge: useAlertsBadge, Body: AlertsDockBody, page: alertsRef() },
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

/** An extension panel as a dock panel: the body page, the count as the
 *  badge, and its `open` (else body) page behind the header's arrow. */
export function extensionDockPanel(panel: ExtensionPanel, extensions: Extension[] | null = null): DockPanelDef {
  return {
    id: `ext:${panel.id}`,
    title: panel.title,
    icon: panel.icon ?? "box",
    page: panelOpenRef(panel, extensions),
    useBadge() {
      const runs = usePanelRuns().runs[panel.id];
      const count = runs?.count;
      return count && count > 0 ? { count } : null;
    },
    Header() {
      const { runs, choose } = usePanelRuns();
      return (
        <>
          {(runs[panel.id]?.choices ?? []).map((c) => (
            <PanelChoiceToggle key={c.name} choice={c} testIdPrefix="dock-panel" onPick={(v) => choose(panel.id, c.name, v)} />
          ))}
        </>
      );
    },
    Collapsed({ onOpenPage, streamId }) {
      const collapsed = usePanelRuns().runs[panel.id]?.collapsed;
      return collapsed ? <PanelCollapsed collapsed={collapsed} testIdPrefix="dock-panel" streamId={streamId} onOpenPage={onOpenPage} /> : null;
    },
    Body({ onOpenPage, streamId }) {
      const page = usePanelRuns().runs[panel.id]?.body;
      return page ? (
        <PanelBody body={page} testIdPrefix="dock-panel" streamId={streamId} onOpenPage={onOpenPage} onMore={() => onOpenPage(panelOpenRef(panel, extensions))} />
      ) : (
        <div style={{ padding: "8px 12px", color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Loading…</div>
      );
    },
  };
}

export function useDockPanels(): DockPanelDef[] {
  const { panels } = usePanelRuns();
  const extensions = useExtensions();
  return useMemo(
    () => orderDockPanels(core, panels.filter((p) => p.dock).map((p) => extensionDockPanel(p, extensions))),
    [panels, extensions],
  );
}
