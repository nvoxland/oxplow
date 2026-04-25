/**
 * Drag-and-drop transport for "Add to agent context" gestures. A
 * separate MIME from the work-item reorder DnD (`WORK_ITEM_DRAG_MIME`)
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

export function setContextRefDrag(e: AnyDragEvent, ref: ContextRef): void {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.setData(CONTEXT_REF_MIME, JSON.stringify(ref));
  // Plain-text fallback so dragging into a non-aware text input still
  // does a sensible thing (e.g. a chat outside the terminal).
  const fallback = ref.kind === "file"
    ? `@${ref.path}`
    : ref.kind === "note"
      ? `@.oxplow/notes/${ref.slug}.md`
      : `[oxplow work-item ${ref.itemId}]`;
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
    if (parsed.kind === "note" && typeof parsed.slug === "string") return { kind: "note", slug: parsed.slug };
    if (parsed.kind === "work-item"
      && typeof parsed.itemId === "string"
      && typeof parsed.title === "string"
      && typeof parsed.status === "string") {
      return { kind: "work-item", itemId: parsed.itemId, title: parsed.title, status: parsed.status };
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
