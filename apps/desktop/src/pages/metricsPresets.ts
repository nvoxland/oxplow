// Saved Explorer views (tsk232). A preset captures the Explorer's control state
// — which measures are overlaid, the group-by, and the viz — under a name, so a
// useful cross-metric view (e.g. "coverage vs complexity by module") can be
// reopened. Persisted in localStorage (same approach as bookmarks / recents),
// so it's per-device and needs no backend.

export type ExplorerPreset = {
  name: string;
  selected: string[];
  groupBy: string;
  viz: string;
};

const KEY = "oxplow.metrics.explorerPresets";

/** A minimal storage shim so the pure load/save logic is testable without a DOM
 *  (tests pass a fake; the app passes `localStorage`). */
export type PresetStore = Pick<Storage, "getItem" | "setItem">;

function backing(store?: PresetStore): PresetStore | null {
  if (store) return store;
  if (typeof localStorage !== "undefined") return localStorage;
  return null;
}

/** All saved presets (empty when none / unreadable / malformed). */
export function loadPresets(store?: PresetStore): ExplorerPreset[] {
  const b = backing(store);
  if (!b) return [];
  try {
    const raw = b.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (p): p is ExplorerPreset =>
        !!p &&
        typeof (p as ExplorerPreset).name === "string" &&
        Array.isArray((p as ExplorerPreset).selected),
    );
  } catch {
    return [];
  }
}

/** Insert or replace `preset` (by name, case-sensitive) and persist. Returns the
 *  new list. A blank name is ignored (returns the list unchanged). */
export function savePreset(
  preset: ExplorerPreset,
  store?: PresetStore,
): ExplorerPreset[] {
  if (!preset.name.trim()) return loadPresets(store);
  const b = backing(store);
  const next = [
    ...loadPresets(store).filter((p) => p.name !== preset.name),
    preset,
  ].sort((a, b2) => a.name.localeCompare(b2.name));
  b?.setItem(KEY, JSON.stringify(next));
  return next;
}

/** Remove the preset named `name` and persist. Returns the new list. */
export function removePreset(name: string, store?: PresetStore): ExplorerPreset[] {
  const b = backing(store);
  const next = loadPresets(store).filter((p) => p.name !== name);
  b?.setItem(KEY, JSON.stringify(next));
  return next;
}
