/// The left nav's panels and the person's layout of them (P6.G1,
/// `.context/pages-and-tabs.md` → "Left-nav panels"): every enabled
/// extension's `panels:` — core has none of its own — ordered, hidden and
/// collapsed as `panel_layout` says. Pure: the rail reads and writes the
/// layout through `get_panel_layout` / `set_panel_layout`.
import type { ExtensionPanel, PanelPlacement } from "../../tauri-bridge/generated/bindings.js";
import { lensRef, refFromTabId } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";

/** Every panel starts open: a collapsed Work hid the task a person had just
 *  added (tsk1045). */
const DEFAULT_COLLAPSED = new Set<string>();

/** An extension panel's id in the layout. */
export function extensionPanelId(panel: ExtensionPanel): string {
  return `ext:${panel.id}`;
}

/** What an extension panel's header opens: the page it names (`open`,
 *  as Comments opens the inbox), else its body lens. */
export function panelOpenRef(panel: ExtensionPanel): TabRef {
  return (panel.open ? refFromTabId(panel.open) : null) ?? lensRef(panel.body);
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

/** The full stored form of a resolved layout (visible first, then hidden),
 *  keeping each placement in `stored` for a panel not available now (an
 *  extension's, before extensions load) right after the entry it followed
 *  there (tsk998): an edit never drops where the person put a panel it
 *  can't see yet. */
function store(layout: ResolvedLayout, stored: PanelPlacement[], available: string[]): PanelPlacement[] {
  const out: PanelPlacement[] = [
    ...layout.order.map((panel) => ({ panel, hidden: false, collapsed: layout.collapsed.has(panel) })),
    ...layout.hidden.map((panel) => ({ panel, hidden: true, collapsed: layout.collapsed.has(panel) })),
  ];
  const known = new Set(available);
  stored.forEach((placement, i) => {
    if (known.has(placement.panel)) return;
    const before = i > 0 ? out.findIndex((p) => p.panel === stored[i - 1]!.panel) : -1;
    out.splice(before + 1, 0, placement);
  });
  return out;
}

/** Move `panel` right `side` of `target` among the visible panels. Relative,
 *  not an index: an edit made before the stored layout loads is replayed
 *  on it, and must land beside the same panel there (tsk998). */
export function movePanelBeside(
  available: string[],
  stored: PanelPlacement[],
  panel: string,
  target: string,
  side: "before" | "after",
): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  const order = layout.order.filter((id) => id !== panel);
  const at = order.indexOf(target);
  if (at < 0 || panel === target) return stored;
  order.splice(side === "before" ? at : at + 1, 0, panel);
  return store({ ...layout, order }, stored, available);
}

export function hidePanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  return store(
    { ...layout, order: layout.order.filter((id) => id !== panel), hidden: [...layout.hidden, panel] },
    stored,
    available,
  );
}

/** Show a hidden panel again, at the bottom. */
export function showPanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  return store(
    { ...layout, order: [...layout.order, panel], hidden: layout.hidden.filter((id) => id !== panel) },
    stored,
    available,
  );
}

/** Collapse or expand `panel`. Absolute, not a toggle: an edit made
 *  before the stored layout loads is replayed on it, and must mean the
 *  same there (tsk972). */
export function setCollapsed(
  available: string[],
  stored: PanelPlacement[],
  panel: string,
  collapsed: boolean,
): PanelPlacement[] {
  const layout = resolveLayout(available, stored);
  const next = new Set(layout.collapsed);
  if (collapsed) next.add(panel);
  else next.delete(panel);
  return store({ ...layout, collapsed: next }, stored, available);
}

/** Make `panel` visible and expanded: shown at the bottom if hidden, and
 *  uncollapsed (the Alerts row that points at Approvals uses it). */
export function revealPanel(available: string[], stored: PanelPlacement[], panel: string): PanelPlacement[] {
  const hidden = resolveLayout(available, stored).hidden.includes(panel);
  const base = hidden ? showPanel(available, stored, panel) : stored;
  const shown = resolveLayout(available, base);
  const collapsed = new Set(shown.collapsed);
  collapsed.delete(panel);
  return store({ ...shown, collapsed }, base, available);
}

/** A change to the stored layout, applied to whatever it is when it lands. */
export type LayoutEdit = (base: PanelPlacement[]) => PanelPlacement[];

export interface LayoutSync {
  /** Apply `change`: shown at once, saved once the stored layout is known. */
  edit(change: LayoutEdit): void;
  /** Stop: nothing more is shown or saved. */
  close(): void;
}

/** The person's layout kept in step with `panel_layout` (tsk972, tsk998):
 *
 *  - Until the stored layout has loaded, edits are shown at once and kept;
 *    once it arrives they're replayed on it and saved — never a write over
 *    a layout not read yet, never a load undoing an edit.
 *  - A load that fails is said (`failed`) and the defaults are shown, but
 *    nothing is saved over the layout it couldn't read: the next edit loads
 *    again, and is replayed on what that load finds.
 *  - Saves go one at a time and the latest layout is the last saved, so an
 *    older save can't land after a newer one.
 *
 *  `ready` is called once, when the first load has answered either way. */
export function layoutSync(io: {
  load(): Promise<PanelPlacement[]>;
  save(layout: PanelPlacement[]): Promise<void>;
  show(layout: PanelPlacement[]): void;
  ready(): void;
  failed(action: string, error: unknown): void;
}): LayoutSync {
  let current: PanelPlacement[] = [];
  let state: "loading" | "failed" | "loaded" = "loading";
  let pending: LayoutEdit[] = [];
  let answered = false;
  let closed = false;
  let saving = false;
  let queued: PanelPlacement[] | null = null;

  const save = (layout: PanelPlacement[]) => {
    if (saving) {
      queued = layout;
      return;
    }
    saving = true;
    void io
      .save(layout)
      .catch((e: unknown) => io.failed("Save the panel layout", e))
      .finally(() => {
        saving = false;
        const next = queued;
        queued = null;
        if (next && !closed) save(next);
      });
  };
  const answer = () => {
    if (answered) return;
    answered = true;
    io.ready();
  };
  const load = () => {
    state = "loading";
    io.load().then(
      (base) => {
        if (closed) return;
        state = "loaded";
        const edits = pending;
        pending = [];
        current = edits.reduce((acc, change) => change(acc), base);
        io.show(current);
        answer();
        if (edits.length > 0) save(current);
      },
      (e: unknown) => {
        if (closed) return;
        state = "failed";
        io.failed("Load the panel layout", e);
        answer();
      },
    );
  };
  load();
  return {
    edit(change) {
      if (closed) return;
      current = change(current);
      io.show(current);
      if (state === "loaded") {
        save(current);
        return;
      }
      pending.push(change);
      if (state === "failed") load();
    },
    close() {
      closed = true;
    },
  };
}
