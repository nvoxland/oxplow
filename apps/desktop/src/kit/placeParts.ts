import type { SlotAt } from "../tauri-bridge/generated/bindings.js";

/**
 * Where a part an extension mounts goes among a page's own parts (a
 * document's sections, a dashboard's tiles): at the start, at the end, or
 * beside one of the page's parts, named by its stable id. The id, not a
 * position, is the contract, so a mount keeps working as the page changes.
 * It matches the manifest's `at:` (`SlotAt` in the bindings).
 */
export type PartAt = SlotAt;

export interface Placed<T> {
  id: string;
  item: T;
}

/** The page's parts in order with the mounts placed among them. Mounts at
 *  one place keep their declared order; one beside a part the page doesn't
 *  have goes at the end (the loader warns about it). Anchors name the
 *  page's own parts only. */
export function placeParts<T>(own: readonly Placed<T>[], mounts: readonly (Placed<T> & { at: PartAt })[]): Placed<T>[] {
  const ownIds = new Set(own.map((p) => p.id));
  const anchor = (at: PartAt): PartAt =>
    typeof at === "string" ? at : ownIds.has("after" in at ? at.after : at.before) ? at : "end";
  const placed = mounts.map((m) => ({ ...m, at: anchor(m.at) }));
  const at = (pred: (a: PartAt) => boolean) => placed.filter((m) => pred(m.at)).map(({ id, item }) => ({ id, item }));
  const out: Placed<T>[] = at((a) => a === "start");
  for (const part of own) {
    out.push(...at((a) => typeof a === "object" && "before" in a && a.before === part.id));
    out.push(part);
    out.push(...at((a) => typeof a === "object" && "after" in a && a.after === part.id));
  }
  out.push(...at((a) => a === "end"));
  return out;
}
