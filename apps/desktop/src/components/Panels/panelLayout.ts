/// The left nav's panels and the person's layout of them (P6.G1,
/// `.context/pages-and-tabs.md` → "Left-nav panels"): core's built-in
/// panels and every enabled extension's `panels:`, ordered, hidden and
/// collapsed as `panel_layout` says. Pure: the rail reads and writes the
/// layout through `get_panel_layout` / `set_panel_layout`.
import type { ExtensionPanel, PanelPlacement } from "../../tauri-bridge/generated/bindings.js";

export interface CorePanel {
  id: string;
  title: string;
}

/** Core's panels, in their default order. */
export const CORE_PANELS: readonly CorePanel[] = [
  { id: "core:alerts", title: "Alerts" },
  { id: "core:approvals", title: "Approvals" },
  { id: "core:uncommitted", title: "Uncommitted" },
  { id: "core:comments", title: "Comments" },
  { id: "core:work", title: "Work" },
  { id: "core:bookmarks", title: "Bookmarks" },
];

/** Work keeps a one-line summary, so it starts collapsed. */
const DEFAULT_COLLAPSED = new Set(["core:work"]);

/** An extension panel's id in the layout. */
export function extensionPanelId(panel: ExtensionPanel): string {
  return `ext:${panel.id}`;
}

export interface ResolvedLayout {
  /** Visible panels, top to bottom. */
  order: string[];
  hidden: string[];
  collapsed: Set<string>;
}

/** The layout for the panels `available` now: the stored order first (ids
 *  no longer available dropped), then any new panel, expanded unless it
 *  defaults collapsed. */
export function resolveLayout(available: string[], stored: PanelPlacement[]): ResolvedLayout {
  const known = new Set(available);
  const kept = stored.filter((p) => known.has(p.panel));
  const seen = new Set(kept.map((p) => p.panel));
  const all = [
    ...kept,
    ...available
      .filter((id) => !seen.has(id))
      .map((id) => ({ panel: id, hidden: false, collapsed: DEFAULT_COLLAPSED.has(id) })),
  ];
  return {
    order: all.filter((p) => !p.hidden).map((p) => p.panel),
    hidden: all.filter((p) => p.hidden).map((p) => p.panel),
    collapsed: new Set(all.filter((p) => p.collapsed).map((p) => p.panel)),
  };
}

/** The full stored form of a resolved layout (visible first, then hidden). */
function store(layout: ResolvedLayout): PanelPlacement[] {
  return [
    ...layout.order.map((panel) => ({ panel, hidden: false, collapsed: layout.collapsed.has(panel) })),
    ...layout.hidden.map((panel) => ({ panel, hidden: true, collapsed: layout.collapsed.has(panel) })),
  ];
}

/** Move `panel` to `index` among the visible panels. */
export function movePanel(available: string[], stored: PanelPlacement[], panel: string, index: number): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  const order = layout.order.filter((id) => id !== panel);
  order.splice(Math.max(0, Math.min(index, order.length)), 0, panel);
  return store({ ...layout, order });
}

export function hidePanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  return store({ ...layout, order: layout.order.filter((id) => id !== panel), hidden: [...layout.hidden, panel] });
}

/** Show a hidden panel again, at the bottom. */
export function showPanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  return store({ ...layout, order: [...layout.order, panel], hidden: layout.hidden.filter((id) => id !== panel) });
}

export function toggleCollapsed(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  const collapsed = new Set(layout.collapsed);
  if (collapsed.has(panel)) collapsed.delete(panel);
  else collapsed.add(panel);
  return store({ ...layout, collapsed });
}

/** Make `panel` visible and expanded: shown at the bottom if hidden, and
 *  uncollapsed (the Alerts row that points at Approvals uses it). */
export function revealPanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const hidden = resolveLayout(available, stored).hidden.includes(panel);
  const shown = resolveLayout(available, hidden ? showPanel(available, stored, panel) : stored);
  const collapsed = new Set(shown.collapsed);
  collapsed.delete(panel);
  return store({ ...shown, collapsed });
}
