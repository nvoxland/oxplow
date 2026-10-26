import { windowStorage } from "./shellStorage.js";

export const CLIENT_ID_KEY = "oxplow-ui-client-id";

/** The same window identity for logs, tab reports and its event socket.
 *  Keep a memory fallback when session storage is unavailable. This module
 *  has no API/transport dependency: the socket can use it during startup. */
export function createUiClientId(
  storage: Pick<typeof windowStorage, "getItem" | "setItem"> = windowStorage,
  generate: () => string = () => globalThis.crypto?.randomUUID?.() ?? `client-${Date.now()}-${Math.random().toString(16).slice(2)}`,
): () => string {
  let id: string | null = null;
  return () => {
    if (id) return id;
    try { id = storage.getItem(CLIENT_ID_KEY); } catch { /* Memory still identifies this window. */ }
    if (!id) {
      id = generate();
      try { storage.setItem(CLIENT_ID_KEY, id); } catch { /* Keep the memory identity. */ }
    }
    return id;
  };
}

export const getUiClientId = createUiClientId();
