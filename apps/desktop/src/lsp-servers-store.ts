/// Renderer-side cache of `list_lsp_servers` — the data-driven
/// replacement for the old hardcoded `isLspCandidateLanguage`. Monaco
/// provider callbacks and menu gating consult `hasLspServer`
/// synchronously; the cache refreshes on `lspServersChanged` (Mason
/// install/remove) and `configChanged` (oxplow.yaml edits).

import {
  listLspServers,
  subscribeOxplowEvents,
  type LspServerListing,
} from "./api.js";
import { logUi } from "./logger.js";

let servers: LspServerListing[] = [];
let loadStarted = false;
let eventsWired = false;
const listeners = new Set<() => void>();

/// Test seam: replace the RPC fetcher (api.js calls hit the Tauri
/// transport, which doesn't exist under bun test).
let fetcher: () => Promise<LspServerListing[]> = listLspServers;
export function _setLspServersFetcherForTests(
  fn: (() => Promise<LspServerListing[]>) | null,
): void {
  fetcher = fn ?? listLspServers;
  servers = [];
  loadStarted = false;
}

export function lspServers(): LspServerListing[] {
  ensureLoaded();
  return servers;
}

export function hasLspServer(languageId: string): boolean {
  ensureLoaded();
  return servers.some((s) => s.languageId === languageId);
}

export async function refreshLspServers(): Promise<void> {
  loadStarted = true;
  try {
    servers = await fetcher();
  } catch (error) {
    logUi("debug", "lsp-servers-store: refresh failed", {
      error: error instanceof Error ? error.message : String(error),
    });
    return;
  }
  for (const listener of listeners) listener();
}

export function subscribeLspServers(listener: () => void): () => void {
  listeners.add(listener);
  ensureLoaded();
  return () => listeners.delete(listener);
}

function ensureLoaded(): void {
  if (!loadStarted) {
    loadStarted = true;
    void refreshLspServers();
  }
  if (!eventsWired) {
    eventsWired = true;
    subscribeOxplowEvents((event) => {
      if (event.kind === "lspServersChanged" || event.kind === "configChanged") {
        void refreshLspServers();
      }
    });
  }
}
