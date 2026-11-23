/// The one owner of the person's left-nav layout for the viewer — the
/// selected thread and its stream (`panel_layout` through
/// `get_panel_layout` / `set_panel_layout`, `panelLayout.ts`): which rail
/// panels show in which group (this thread's, this stream's, the
/// project's), in what order, collapsed or not. The rail and the View ▸
/// Left Rail menu read and change the same layout. Provided inside
/// `PanelRunsProvider` (it reads the panels from there).

import type { ReactNode } from "react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";

import { getPanelLayout, setPanelLayout } from "../../api.js";
import { recordOpError } from "../../opErrors.js";
import type { ScopedPanelLayout } from "../../tauri-bridge/generated/bindings.js";
import {
  EMPTY_LAYOUT,
  extensionPanelId,
  hidePanel,
  layoutSync,
  movePanelBeside,
  movePanelToGroup,
  resolveLayout,
  setCollapsed,
  showPanel,
  type LayoutEdit,
  type LayoutSync,
  type PanelGroup,
  type ResolvedLayout,
} from "./panelLayout.js";
import { usePanelRuns } from "./PanelRunsContext.js";

export interface PanelLayoutValue {
  /** The rail's panels now (a dock panel isn't one), by layout id. */
  available: string[];
  /** A rail panel's title, by layout id. */
  titleOf(id: string): string;
  layout: ResolvedLayout;
  /** The stored layout has arrived: what shows is the person's layout. */
  loaded: boolean;
  /** The groups this viewer has: no thread, no thread group. */
  groups: PanelGroup[];
  setCollapsed(id: string, collapsed: boolean): void;
  hide(id: string): void;
  /** Show a hidden panel at the end of the project's group. */
  show(id: string): void;
  moveBeside(id: string, target: string, side: "before" | "after"): void;
  moveToGroup(id: string, group: PanelGroup): void;
}

const PanelLayoutContext = createContext<PanelLayoutValue | null>(null);

export function PanelLayoutProvider({
  streamId,
  threadId,
  children,
}: {
  streamId: string | null;
  threadId: string | null;
  children: ReactNode;
}) {
  const { panels } = usePanelRuns();
  const railPanels = useMemo(() => panels.filter((p) => !p.dock), [panels]);
  const available = useMemo(() => railPanels.map(extensionPanelId), [railPanels]);
  const titleOf = useCallback(
    (id: string) => railPanels.find((p) => extensionPanelId(p) === id)?.title ?? id,
    [railPanels],
  );
  const [stored, setStored] = useState<ScopedPanelLayout>(EMPTY_LAYOUT);
  const [loaded, setLoaded] = useState(false);
  // Kept in step with the viewer's lists by `layoutSync`: an edit before
  // the load is replayed on it, a failed load saves nothing, saves land in
  // order. Another viewer is another sync.
  const sync = useRef<LayoutSync | null>(null);
  useEffect(() => {
    setLoaded(false);
    const s = layoutSync({
      load: () => getPanelLayout(streamId, threadId),
      save: (layout) => setPanelLayout(streamId, threadId, layout),
      show: setStored,
      ready: () => setLoaded(true),
      failed: (label, e) => recordOpError({ label, message: e instanceof Error ? e.message : String(e) }),
    });
    sync.current = s;
    return () => {
      s.close();
      sync.current = null;
    };
  }, [streamId, threadId]);
  const layout = useMemo(() => resolveLayout(available, stored), [available, stored]);
  const edit = useCallback((change: LayoutEdit) => sync.current?.edit(change), []);
  const value = useMemo<PanelLayoutValue>(
    () => ({
      available,
      titleOf,
      layout,
      loaded,
      groups: threadId ? ["thread", "stream", "project"] : streamId ? ["stream", "project"] : ["project"],
      setCollapsed: (id, collapsed) => edit((base) => setCollapsed(available, base, id, collapsed)),
      hide: (id) => edit((base) => hidePanel(available, base, id)),
      show: (id) => edit((base) => showPanel(available, base, id)),
      moveBeside: (id, target, side) => edit((base) => movePanelBeside(available, base, id, target, side)),
      moveToGroup: (id, group) => edit((base) => movePanelToGroup(available, base, id, group)),
    }),
    [available, titleOf, layout, loaded, threadId, streamId, edit],
  );
  return <PanelLayoutContext.Provider value={value}>{children}</PanelLayoutContext.Provider>;
}

/** The viewer's layout; outside a provider, nothing can change it. */
export function usePanelLayout(): PanelLayoutValue {
  const value = useContext(PanelLayoutContext);
  if (!value) throw new Error("usePanelLayout needs a PanelLayoutProvider");
  return value;
}
