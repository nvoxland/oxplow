/// The left nav's panels and the person's layout of them (P6.G1,
/// `.context/pages-and-tabs.md` → "Left-nav panels"): every enabled
/// extension's `panels:` — core has none of its own — in three groups, the
/// selected thread's, its stream's and the project's, ordered, hidden and
/// collapsed as `panel_layout` says for that viewer. Pure: the rail reads
/// and writes the viewer's lists through `get_panel_layout` /
/// `set_panel_layout`.
import type { Extension, ExtensionPanel, PanelPlacement, ScopedPanelLayout } from "../../tauri-bridge/generated/bindings.js";
import { VISIBILITY_SCOPES, type VisibilityScope } from "../../visibilityScope.js";
import { pageFileOf, pageFileUri } from "../../pages/pageFiles.js";
import { refFromTabId } from "../../tabs/pageRefs.js";
import { extPageUri } from "../../uri/uri.js";
import type { TabRef } from "../../tabs/tabState.js";

/** Every panel starts open: a collapsed Work hid the task a person had just
 *  added. */
const DEFAULT_COLLAPSED = new Set<string>();

/** No lists yet: what shows before the stored layout loads. */
export const EMPTY_LAYOUT: ScopedPanelLayout = { thread: [], stream: [], project: [] };

/** An extension panel's id in the layout. */
export function extensionPanelId(panel: ExtensionPanel): string {
  return `ext:${panel.id}`;
}

/** What an extension panel's header opens: the page it names (`open`,
 *  as Alerts opens its page), else its body page. */
export function panelOpenRef(panel: ExtensionPanel, extensions: Extension[] | null = null): TabRef {
  const named = panel.open ? refFromTabId(panel.open) : null;
  if (named) return named;
  // The body is a page id, `<namespace>/<slug>`.
  const slash = panel.body.indexOf("/");
  const route = { namespace: panel.body.slice(0, slash), page: panel.body.slice(slash + 1), params: {} };
  const body = pageFileOf(extensions, route);
  return refFromTabId(body ? pageFileUri(extensions ?? [], body) : "") ?? { id: extPageUri(panel.body.slice(0, slash), panel.body.slice(slash + 1)) };
}

/** A rail group: the selected thread's panels, its stream's, the project's. */
export type PanelGroup = VisibilityScope;

export interface ResolvedLayout {
  /** Shown panels per group, top to bottom. */
  groups: Record<PanelGroup, string[]>;
  /** Shown panels, top to bottom: the thread's, the stream's, the project's. */
  order: string[];
  /** The group a shown panel is in. */
  groupOf: Map<string, PanelGroup>;
  /** Available panels this viewer doesn't show. */
  hidden: string[];
  collapsed: Set<string>;
}

/** The layout for the panels `available` now: each list in its stored
 *  order (ids no longer available dropped), then any panel no list names
 *  at the end of the project's, expanded unless it defaults collapsed. A
 *  panel the project's list has hidden — placed on another thread or
 *  stream, or hidden — isn't new: it's hidden here. */
export function resolveLayout(available: string[], stored: ScopedPanelLayout): ResolvedLayout {
  const known = new Set(available);
  const groups: Record<PanelGroup, string[]> = { thread: [], stream: [], project: [] };
  const groupOf = new Map<string, PanelGroup>();
  const collapsed = new Set<string>();
  const named = new Set<string>();
  for (const group of VISIBILITY_SCOPES) {
    for (const p of stored[group]) {
      named.add(p.panel);
      if (!known.has(p.panel) || p.hidden || groupOf.has(p.panel)) continue;
      groups[group].push(p.panel);
      groupOf.set(p.panel, group);
      if (p.collapsed) collapsed.add(p.panel);
    }
  }
  for (const id of available) {
    if (named.has(id)) continue;
    groups.project.push(id);
    groupOf.set(id, "project");
    if (DEFAULT_COLLAPSED.has(id)) collapsed.add(id);
  }
  return {
    groups,
    order: VISIBILITY_SCOPES.flatMap((g) => groups[g]),
    groupOf,
    hidden: available.filter((id) => !groupOf.has(id)),
    collapsed,
  };
}

/** `stored` with every available panel no list names in the project's,
 *  shown at its end: what `resolveLayout` draws, so an edit can stand
 *  beside a new panel. A placement of a panel not available now (an
 *  extension's, before extensions load) stays where it is. */
function named(available: string[], stored: ScopedPanelLayout): ScopedPanelLayout {
  const has = new Set(VISIBILITY_SCOPES.flatMap((g) => stored[g].map((p) => p.panel)));
  const added = available.filter((id) => !has.has(id)).map((panel) => ({ panel, hidden: false, collapsed: DEFAULT_COLLAPSED.has(panel) }));
  return added.length === 0 ? stored : { ...stored, project: [...stored.project, ...added] };
}

/** `panel` out of the viewer's lists: gone from the thread's and the
 *  stream's, hidden in the project's (it keeps its row: placed, not new). */
function without(stored: ScopedPanelLayout, panel: string): ScopedPanelLayout {
  const inProject = stored.project.some((p) => p.panel === panel);
  return {
    thread: stored.thread.filter((p) => p.panel !== panel),
    stream: stored.stream.filter((p) => p.panel !== panel),
    project: inProject
      ? stored.project.map((p) => (p.panel === panel ? { ...p, hidden: true } : p))
      : [...stored.project, { panel, hidden: true, collapsed: false }],
  };
}

/** The viewer's row of a shown `panel`, if any. */
function shownRow(stored: ScopedPanelLayout, panel: string): PanelPlacement | undefined {
  for (const g of VISIBILITY_SCOPES) {
    const row = stored[g].find((p) => p.panel === panel && !p.hidden);
    if (row) return row;
  }
  return undefined;
}

/** Put `panel` into `group` at `at` (an index into that list, or its end),
 *  out of every other list the viewer sees. In the project's, its hidden
 *  row moves there and shows. */
function place(stored: ScopedPanelLayout, panel: string, group: PanelGroup, at: (list: PanelPlacement[]) => number): ScopedPanelLayout {
  const collapsed = shownRow(stored, panel)?.collapsed ?? false;
  const out = without(stored, panel);
  const list = group === "project" ? out.project.filter((p) => p.panel !== panel) : out[group];
  const i = Math.max(0, Math.min(at(list), list.length));
  const next = [...list.slice(0, i), { panel, hidden: false, collapsed }, ...list.slice(i)];
  return { ...out, [group]: next };
}

/** Move `panel` right `side` of `target`, into `target`'s group. Relative,
 *  not an index: an edit made before the stored layout loads is replayed
 *  on it, and must land beside the same panel there. */
export function movePanelBeside(
  available: string[],
  stored: ScopedPanelLayout,
  panel: string,
  target: string,
  side: "before" | "after",
): ScopedPanelLayout {
  if (panel === target) return stored;
  const base = named(available, stored);
  const group = resolveLayout(available, base).groupOf.get(target);
  if (!group) return stored;
  return place(base, panel, group, (list) => {
    const i = list.findIndex((p) => p.panel === target);
    return side === "before" ? i : i + 1;
  });
}

/** Move `panel` to the end of `group` (a drop on its divider, Show in). */
export function movePanelToGroup(available: string[], stored: ScopedPanelLayout, panel: string, group: PanelGroup): ScopedPanelLayout {
  return place(named(available, stored), panel, group, (list) => list.length);
}

/** Take the panel out of the rail here. */
export function hidePanel(available: string[], stored: ScopedPanelLayout, panel: string): ScopedPanelLayout {
  return without(named(available, stored), panel);
}

/** Show a hidden panel again, at the end of the project's group. */
export function showPanel(available: string[], stored: ScopedPanelLayout, panel: string): ScopedPanelLayout {
  return movePanelToGroup(available, stored, panel, "project");
}

/** Collapse or expand `panel`. Absolute, not a toggle: an edit made
 *  before the stored layout loads is replayed on it, and must mean the
 *  same there. */
export function setCollapsed(available: string[], stored: ScopedPanelLayout, panel: string, collapsed: boolean): ScopedPanelLayout {
  const base = named(available, stored);
  const group = resolveLayout(available, base).groupOf.get(panel);
  if (!group) return base;
  return { ...base, [group]: base[group].map((p) => (p.panel === panel && !p.hidden ? { ...p, collapsed } : p)) };
}

/** Make `panel` visible and expanded: shown at the end of the project's
 *  group if hidden, and uncollapsed. */
export function revealPanel(available: string[], stored: ScopedPanelLayout, panel: string): ScopedPanelLayout {
  const hidden = resolveLayout(available, named(available, stored)).hidden.includes(panel);
  const base = hidden ? showPanel(available, stored, panel) : stored;
  return setCollapsed(available, base, panel, false);
}

/** A change to the stored layout, applied to whatever it is when it lands. */
export type LayoutEdit = (base: ScopedPanelLayout) => ScopedPanelLayout;

export interface LayoutSync {
  /** Apply `change`: shown at once, saved once the stored layout is known. */
  edit(change: LayoutEdit): void;
  /** Stop: nothing more is shown or saved. */
  close(): void;
}

/** The person's layout kept in step with `panel_layout`:
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
  load(): Promise<ScopedPanelLayout>;
  save(layout: ScopedPanelLayout): Promise<void>;
  show(layout: ScopedPanelLayout): void;
  ready(): void;
  failed(action: string, error: unknown): void;
}): LayoutSync {
  let current: ScopedPanelLayout = EMPTY_LAYOUT;
  let state: "loading" | "failed" | "loaded" = "loading";
  let pending: LayoutEdit[] = [];
  let answered = false;
  let closed = false;
  let saving = false;
  let queued: ScopedPanelLayout | null = null;

  const save = (layout: ScopedPanelLayout) => {
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
