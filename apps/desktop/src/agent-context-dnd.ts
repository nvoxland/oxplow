/**
 * Drag-and-drop transport for "Add to agent context" gestures. A
 * separate MIME from the tasks reorder DnD (`TASK_DRAG_MIME`)
 * so the existing reorder logic ignores our payload and vice versa.
 *
 * Drag sources call `setContextRefDrag(e, ref)` in `onDragStart`; the
 * TerminalPane drop handler calls `readContextRef(e)` in `onDragOver`
 * and `onDrop` to recognize the payload.
 */

import type { DragEvent as ReactDragEvent } from "react";
import type { ContextRef } from "./agent-context-ref.js";

type AnyDragEvent = ReactDragEvent | DragEvent;

export const CONTEXT_REF_MIME = "application/x-oxplow-context-ref";

/**
 * MIME type carried by the tasks reorder DnD (defined in
 * `ThreadRail.tsx`). Re-declared here as a constant so this module can
 * decode multi-payload tasks drags without pulling in the React
 * tree. The actual MIME string MUST match `TASK_DRAG_MIME` —
 * tests guard against drift.
 */
export const WORK_ITEM_DRAG_MIME_VALUE = "application/x-oxplow-task";

export function setContextRefDrag(e: AnyDragEvent, ref: ContextRef): void {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.setData(CONTEXT_REF_MIME, JSON.stringify(ref));
  // Plain-text fallback so dragging into a non-aware text input still
  // does a sensible thing (e.g. a chat outside the terminal).
  const fallback = ref.kind === "file"
    ? `@${ref.path}`
    : ref.kind === "wiki"
      ? `@.oxplow/wiki/${ref.slug}.md`
      : `[oxplow task ${ref.itemId}]`;
  dt.setData("text/plain", fallback);
  dt.effectAllowed = "copy";
}

export function readContextRef(e: AnyDragEvent): ContextRef | null {
  const dt = e.dataTransfer;
  if (!dt) return null;
  // Some browsers only expose `types` (not `getData`) during dragover.
  // We probe types first and only call getData on drop where the spec
  // guarantees access.
  const hasMime = Array.from(dt.types ?? []).includes(CONTEXT_REF_MIME);
  if (!hasMime) return null;
  let raw: string;
  try {
    raw = dt.getData(CONTEXT_REF_MIME);
  } catch {
    // dragover restrictions: getData may throw. Treat as "yes, payload
    // is present, but we can't read it yet" — caller still calls
    // preventDefault to keep the drop active.
    return { kind: "file", path: "" }; // sentinel: caller only checks non-null
  }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    if (parsed.kind === "file" && typeof parsed.path === "string") return { kind: "file", path: parsed.path };
    if (parsed.kind === "wiki" && typeof parsed.slug === "string") return { kind: "wiki", slug: parsed.slug };
    if (parsed.kind === "task"
      && typeof parsed.itemId === "string"
      && typeof parsed.title === "string"
      && typeof parsed.status === "string") {
      return { kind: "task", itemId: parsed.itemId, title: parsed.title, status: parsed.status };
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Lightweight check used in `onDragOver` (where `getData` is restricted).
 * Returns true iff the drag payload includes our MIME type.
 */
export function dragHasContextRef(e: AnyDragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes(CONTEXT_REF_MIME);
}

/**
 * Returns true iff the drag payload includes a tasks DnD payload
 * (the multi-id reorder MIME). The agent terminal accepts these for the
 * "drag a marked tasks row onto the agent" gesture — each id
 * resolves to a `tasks` context ref.
 */
export function dragHasTaskRefs(e: AnyDragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes(WORK_ITEM_DRAG_MIME_VALUE);
}

/**
 * Decode a `TASK_DRAG_MIME` payload into the list of tasks ids
 * it carries. Accepts both the multi-id `itemIds` form and the single
 * `itemId` legacy form. Returns `[]` for any malformed payload so
 * callers can `return` cleanly without nested try/catch.
 *
 * Pure — exported for tests.
 */
export function decodeTaskDragPayload(raw: string | null | undefined): string[] {
  if (!raw) return [];
  let parsed: { itemId?: unknown; itemIds?: unknown };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!parsed || typeof parsed !== "object") return [];
  const ids: string[] = [];
  if (Array.isArray(parsed.itemIds)) {
    for (const id of parsed.itemIds) {
      if (typeof id === "string" && id) ids.push(id);
    }
  }
  if (ids.length === 0 && typeof parsed.itemId === "string" && parsed.itemId) {
    ids.push(parsed.itemId);
  }
  return ids;
}

/**
 * Decode the optional `items: [{id,title,status}, …]` slice of a
 * `TASK_DRAG_MIME` payload. Drag sources that have visibility into
 * the tasks record can include this so cross-pane drop targets
 * (e.g. the agent terminal) don't need to look up titles themselves.
 *
 * Returns the resolved `ContextRef[]` directly. Falls back to `[]` when
 * the payload is missing the slice or the entries are malformed.
 *
 * Pure — exported for tests.
 */
export function decodeTaskDragRefs(raw: string | null | undefined): ContextRef[] {
  if (!raw) return [];
  let parsed: { items?: unknown };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!parsed || typeof parsed !== "object") return [];
  if (!Array.isArray(parsed.items)) return [];
  const out: ContextRef[] = [];
  for (const entry of parsed.items) {
    if (!entry || typeof entry !== "object") continue;
    const e = entry as { id?: unknown; title?: unknown; status?: unknown };
    const id = typeof e.id === "string" ? e.id : String(e.id ?? "");
    if (!id) continue;
    if (typeof e.title !== "string") continue;
    if (typeof e.status !== "string") continue;
    out.push({ kind: "task", itemId: id, title: e.title, status: e.status });
  }
  return out;
}

/**
 * Resolve a list of tasks ids into `ContextRef`s by looking up each
 * id in `lookup`. Ids that don't resolve are skipped (the user dragged
 * a row whose data the agent terminal doesn't have visibility into —
 * silently dropping is friendlier than throwing).
 *
 * Pure — exported for tests.
 */
export function resolveTaskContextRefs(
  ids: string[],
  lookup: (id: string) => { title: string; status: string } | null,
): ContextRef[] {
  const out: ContextRef[] = [];
  for (const id of ids) {
    const hit = lookup(id);
    if (!hit) continue;
    out.push({ kind: "task", itemId: id, title: hit.title, status: hit.status });
  }
  return out;
}
