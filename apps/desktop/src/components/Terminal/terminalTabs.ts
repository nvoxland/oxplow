/**
 * Pure helpers for the Terminal page's per-stream terminal list. Kept
 * free of React/DOM so the add / close / rename / persistence logic is
 * unit-testable (see `terminalTabs.test.ts`).
 *
 * The FIRST terminal in every stream uses the sentinel id
 * {@link DEFAULT_TERMINAL_ID}. It maps to the legacy bare `"shell"`
 * `pane_target` and the legacy `stream.id` comment target, so the
 * single shell + any comments that existed before multi-terminal support
 * keep working with zero migration. Additional terminals get generated
 * ids → `shell:<id>` pane target + `<streamId>:<id>` comment target.
 */
export interface TerminalTab {
  id: string;
  title: string;
}

export const DEFAULT_TERMINAL_ID = "default";

/** A fresh single-terminal list — the state a stream starts in. */
export function defaultTerminalList(): TerminalTab[] {
  return [{ id: DEFAULT_TERMINAL_ID, title: "Terminal 1" }];
}

/** Backend `pane_target` for a terminal id. */
export function paneTargetFor(id: string): string {
  return id === DEFAULT_TERMINAL_ID ? "shell" : `shell:${id}`;
}

/** Comment-layer `targetId` for a terminal within a stream. */
export function commentTargetFor(streamId: string, id: string): string {
  return id === DEFAULT_TERMINAL_ID ? streamId : `${streamId}:${id}`;
}

/**
 * Next auto-number for a "Terminal N" title: one past the highest
 * existing numbered title, and never below `list.length + 1`, so the
 * generated title never collides with an existing one even after
 * renames.
 */
function nextTerminalNumber(list: TerminalTab[]): number {
  let maxNumbered = 0;
  for (const t of list) {
    const m = /^Terminal (\d+)$/.exec(t.title.trim());
    if (m) maxNumbered = Math.max(maxNumbered, Number(m[1]));
  }
  return Math.max(maxNumbered, list.length) + 1;
}

/**
 * Append a new terminal with the caller-supplied (already-unique) id and
 * an auto-numbered title. Returns the new list plus the id to activate.
 */
export function addTerminal(
  list: TerminalTab[],
  newId: string,
): { list: TerminalTab[]; activeId: string } {
  const tab: TerminalTab = { id: newId, title: `Terminal ${nextTerminalNumber(list)}` };
  return { list: [...list, tab], activeId: newId };
}

/**
 * Remove a terminal. If it was active, the neighbor (previous, else next)
 * becomes active. Closing the last remaining terminal re-seeds a fresh
 * default so the page is never empty.
 */
export function closeTerminal(
  list: TerminalTab[],
  activeId: string,
  id: string,
): { list: TerminalTab[]; activeId: string } {
  const idx = list.findIndex((t) => t.id === id);
  if (idx === -1) return { list, activeId };

  const next = list.filter((t) => t.id !== id);
  if (next.length === 0) {
    const seeded = defaultTerminalList();
    return { list: seeded, activeId: seeded[0].id };
  }
  if (activeId !== id) return { list: next, activeId };

  // The active terminal closed — pick the previous sibling, else the next.
  const neighbor = next[Math.max(0, idx - 1)] ?? next[0];
  return { list: next, activeId: neighbor.id };
}

/** Rename a terminal. An empty/whitespace title is ignored (no-op). */
export function renameTerminal(list: TerminalTab[], id: string, title: string): TerminalTab[] {
  const trimmed = title.trim();
  if (!trimmed) return list;
  return list.map((t) => (t.id === id ? { ...t, title: trimmed } : t));
}

/**
 * Coerce a persisted (untrusted) value into a valid terminal list:
 * keep only `{id, title}` string entries, drop empty-id/title and
 * duplicate ids, and fall back to a fresh default when nothing survives.
 */
export function normalizeTerminalList(raw: unknown): TerminalTab[] {
  if (!Array.isArray(raw)) return defaultTerminalList();
  const seen = new Set<string>();
  const out: TerminalTab[] = [];
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const { id, title } = entry as { id?: unknown; title?: unknown };
    if (typeof id !== "string" || typeof title !== "string") continue;
    if (!id || !title.trim() || seen.has(id)) continue;
    seen.add(id);
    out.push({ id, title });
  }
  return out.length > 0 ? out : defaultTerminalList();
}
