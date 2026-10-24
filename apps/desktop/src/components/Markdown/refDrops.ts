/**
 * What a drop onto the markdown editor names, and what a paste is
 * (`.context/markdown.md` "Drop and paste"): an "Add to agent context" drag
 * (`CONTEXT_REF_MIME`: a file, a wiki page, a lens row's ref, a work item)
 * or a work-item drag (`WORK_ITEM_DRAG_MIME`, several items) names refs the
 * editor puts in as links (`refLinkOf`); a pasted oxplow URI is a ref, a
 * pasted web URL a link for the selection. Pure.
 */
import { decodeContextRef, decodeWorkItemDrag } from "../../agent-context-dnd.js";
import type { ContextRef } from "../../agent-context-ref.js";
import { CONTEXT_REF_MIME, WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { fileUri, parseUri, wikiUri } from "../../uri/uri.js";

/** A ref to put in: its URI and what it says when it has no bare form. */
export interface DroppedRef {
  uri: string;
  title: string;
}

/** The part of a `DataTransfer` a drop reads. */
export interface DropData {
  readonly types: readonly string[];
  getData(type: string): string;
}

/** The refs a drop names, in order; none for a drop of anything else. */
export function droppedRefs(data: DropData): DroppedRef[] {
  const types = Array.from(data.types);
  if (types.includes(WORK_ITEM_DRAG_MIME)) {
    return (decodeWorkItemDrag(data.getData(WORK_ITEM_DRAG_MIME))?.items ?? []).map((i) => ({ uri: i.ref, title: i.title }));
  }
  if (types.includes(CONTEXT_REF_MIME)) {
    const ref = contextRefUri(decodeContextRef(data.getData(CONTEXT_REF_MIME)));
    return ref ? [ref] : [];
  }
  return [];
}

/** Whether a drag carries refs the editor takes (readable in `dragover`,
 *  where `getData` isn't). */
export function dragCarriesRefs(types: readonly string[]): boolean {
  return types.includes(CONTEXT_REF_MIME) || types.includes(WORK_ITEM_DRAG_MIME);
}

function contextRefUri(ref: ContextRef | null): DroppedRef | null {
  if (!ref) return null;
  switch (ref.kind) {
    case "file":
      return ref.path ? { uri: fileUri(ref.path), title: ref.path } : null;
    case "wiki":
      return { uri: wikiUri(ref.slug), title: ref.slug };
    case "work_item":
      return { uri: ref.ref, title: ref.title };
    case "ref":
      return { uri: ref.ref, title: ref.ref };
    default:
      // A lens with params has no URI to link.
      return null;
  }
}

/** What pasted text is when it is only a link: an oxplow URI (a ref) or a
 *  web URL; null for any other text. */
export function pastedLink(text: string): { kind: "ref"; uri: string } | { kind: "url"; url: string } | null {
  const t = text.trim();
  if (!t || /\s/.test(t)) return null;
  if (t.startsWith("oxplow:")) return parseUri(t) ? { kind: "ref", uri: t } : null;
  return /^https?:\/\/\S+$/i.test(t) ? { kind: "url", url: t } : null;
}
