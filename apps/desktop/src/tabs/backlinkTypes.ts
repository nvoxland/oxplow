/**
 * Renderer shape for one entry in the Backlinks (and Outbound)
 * panel. Decoupled from the SQLite edge row so the renderer can
 * stay kind-agnostic — the IPC reader-mapper turns each
 * `BacklinkEdge` into the page + label+subtitle the row needs.
 */
import type { TabRef } from "./tabState.js";

export interface BacklinkEntry {
  /** The page the row opens; null when no page shows the ref — either
   *  nothing resolves it (`dead`) or it resolves but has no page. */
  ref: TabRef | null;
  /** The far end's canonical ref, as the graph stores it. */
  target: string;
  label: string;
  subtitle?: string;
  /** Nothing resolves the ref (its extension is gone, its record
   *  deleted): shown as its raw text, muted. */
  dead?: boolean;
  /** The work-item link type, when the edge is a work item's link. */
  linkType?: string;
}
