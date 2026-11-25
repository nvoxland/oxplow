import { newId } from "./newId.js";

/** The window's id for one page load: its logs, tab reports, client host and
 *  event socket (reconnects included) share it. It is kept in memory only,
 *  so a reload is a new window to the daemon (a late unregister from the
 *  page that left cannot remove the one that took its place) and a
 *  duplicated browser tab is never the window it came from. This module has
 *  no API/transport dependency: the socket can use it during startup. */
export function createUiClientId(generate: () => string = newId): () => string {
  let id: string | null = null;
  return () => (id ??= generate());
}

export const getUiClientId = createUiClientId();
