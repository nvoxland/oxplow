/**
 * What a tab shows: a page's kind and its ref. The navigation model —
 * tabs, their history, the active one — is `tabs/session.ts`.
 */

/**
 * The kinds of content a tab renders. Every tab id is a canonical ref
 * (`.context/refs.md`). An **entity** page's id is the entity's own ref
 * (`work_item:oxplow:tsk42`, `file:src/a.rs@git:HEAD`), and its kind is
 * the ref's kind. Every other page is a **declared page** — a `pages:`
 * entry of an extension's manifest, oxplow's own included — whose id is
 * `page:<id>[?params]` (oxplow's) or `page:<extension>/<id>[?params]`, and
 * whose kind is `page`: what draws it comes from its declaration. Pages
 * never appear in `page_ref`.
 */
export type EntityPageKind =
  | "file"
  | "dir"
  | "wiki"
  | "work_item"
  | "commit"
  | "metric"
  | "lens"
  // A capture, an effort and an agent turn render as their diff (P2.11).
  | "snapshot"
  | "effort"
  | "turn"
  // Opens its file at the symbol's line; never a tab of its own (P6.E3).
  | "symbol"
  // An agent slot on the thread: its terminal.
  | "agent_session";

export type PageKind = EntityPageKind | "page";

/** Reference to a tab. `id` must be unique across page kinds within a thread. */
export interface TabRef {
  id: string;
  kind: PageKind;
  /** Page-kind-specific data — file path, tasks id, dashboard variant, etc. */
  payload: unknown;
  /** Pinned: it leads the strip, isn't dragged, and is never evicted or
   *  counted against the tab cap (an agent session's tab). */
  pinned?: true;
}
