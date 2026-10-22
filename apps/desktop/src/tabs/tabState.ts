/**
 * What a tab shows: a URI (`.context/resources.md`). The navigation model —
 * tabs, their history, the active one — is `tabs/session.ts`; what draws a
 * tab is the page that renders its URI (`pages/renderers.ts`), or a route's
 * declaration (`oxplow:oxplow/page?page=…`).
 */

/** Reference to a tab: its URI, unique within a thread's tabs. */
export interface TabRef {
  /** The URI it shows: what (`?`) and how (`#`, its view). */
  id: string;
  /** Pinned: it leads the strip, isn't dragged, and is never evicted or
   *  counted against the tab cap (an agent session's tab). */
  pinned?: true;
}
