/**
 * Drag-and-drop transport for "Add to agent context" gestures. A
 * separate MIME from the work-item drag (`WORK_ITEM_DRAG_MIME`)
 * so the list's reorder logic ignores our payload and vice versa.
 * Both MIMEs come from `dragMimes.ts`, which is import-free precisely so
 * this decoder can name them without pulling in the React tree.
 *
 * Drag sources call `setContextRefDrag(e, ref)` in `onDragStart`; the
 * TerminalPane drop handler calls `readContextRef(e)` in `onDragOver`
 * and `onDrop` to recognize the payload.
 */

import type { DragEvent as ReactDragEvent } from "react";
import { formatContextMention, type ContextRef } from "./agent-context-ref.js";
import { CONTEXT_REF_MIME, WORK_ITEM_DRAG_MIME } from "./dragMimes.js";
import { parseRef } from "./refs/ref.js";

type AnyDragEvent = ReactDragEvent | DragEvent;



export function setContextRefDrag(e: AnyDragEvent, ref: ContextRef): void {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.setData(CONTEXT_REF_MIME, JSON.stringify(ref));
  // Plain-text fallback so dragging into a non-aware text input still
  // does a sensible thing (e.g. a chat outside the terminal).
  dt.setData("text/plain", formatContextMention(ref).trimEnd());
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
  return decodeContextRef(raw);
}

/**
 * Decode a `CONTEXT_REF_MIME` payload: a file, wiki page, work item, or any
 * canonical ref (what a lens row drags). Null for anything
 * malformed. Pure — exported for tests.
 */
export function decodeContextRef(raw: string | null | undefined): ContextRef | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    if (parsed.kind === "file" && typeof parsed.path === "string") return { kind: "file", path: parsed.path };
    if (parsed.kind === "wiki" && typeof parsed.slug === "string") return { kind: "wiki", slug: parsed.slug };
    if (parsed.kind === "ref" && typeof parsed.ref === "string" && parseRef(parsed.ref)) return { kind: "ref", ref: parsed.ref };
    if (parsed.kind === "work_item"
      && typeof parsed.ref === "string"
      && typeof parsed.title === "string"
      && typeof parsed.state === "string") {
      return { kind: "work_item", ref: parsed.ref, title: parsed.title, state: parsed.state };
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

/** A work-item drag: the refs it carries (several when marked rows move
 *  together), each one's title and state so a drop target (the agent
 *  terminal) needs no lookup, the list it came from, and the epic when
 *  it came out of one. */
export interface WorkItemDrag {
  refs: string[];
  items: Array<{ ref: string; title: string; state: string }>;
  fromThreadId: string | null;
  parentEpicRef?: string;
}

/** Start a work-item drag. */
export function setWorkItemDrag(e: AnyDragEvent, drag: WorkItemDrag): void {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.effectAllowed = "move";
  dt.setData("text/plain", drag.refs.join(" "));
  dt.setData(WORK_ITEM_DRAG_MIME, JSON.stringify(drag));
}

/**
 * True iff the drag carries work items. Usable in `onDragOver`, where
 * `getData` is restricted.
 */
export function dragHasWorkItems(e: AnyDragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes(WORK_ITEM_DRAG_MIME);
}

/**
 * Decode a `WORK_ITEM_DRAG_MIME` payload; null for anything malformed.
 * Entries that aren't well formed are dropped. Pure — exported for tests.
 */
export function decodeWorkItemDrag(raw: string | null | undefined): WorkItemDrag | null {
  if (!raw) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
  const p = parsed as Record<string, unknown>;
  const refs = Array.isArray(p.refs) ? p.refs.filter((r): r is string => typeof r === "string" && r !== "") : [];
  const items = Array.isArray(p.items)
    ? p.items.flatMap((e) => {
        if (!e || typeof e !== "object") return [];
        const { ref, title, state } = e as Record<string, unknown>;
        return typeof ref === "string" && typeof title === "string" && typeof state === "string" ? [{ ref, title, state }] : [];
      })
    : [];
  return {
    refs,
    items,
    fromThreadId: typeof p.fromThreadId === "string" ? p.fromThreadId : null,
    ...(typeof p.parentEpicRef === "string" ? { parentEpicRef: p.parentEpicRef } : {}),
  };
}

/** The context refs a work-item drag adds to the agent: one per item it
 *  resolved. */
export function workItemDragRefs(drag: WorkItemDrag | null): ContextRef[] {
  return (drag?.items ?? []).map((i) => ({ kind: "work_item" as const, ref: i.ref, title: i.title, state: i.state }));
}
