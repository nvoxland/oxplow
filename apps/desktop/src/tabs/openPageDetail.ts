/**
 * Page-specific context a page publishes about itself (a lens's id and
 * current params), keyed by page id. App reports the *active* page plus
 * its detail to the backend (`report_open_page`), so an agent's
 * `get_open_page` sees what the human sees. Keeping detail here, rather
 * than having pages report directly, means a mounted-but-hidden page can
 * never overwrite what's reported for the active one.
 */
type Detail = Record<string, unknown>;

export interface PageDetailStore {
  publish(pageId: string, detail: Detail | null): void;
  get(pageId: string): Detail | null;
  subscribe(fn: () => void): () => void;
}

export function createPageDetailStore(): PageDetailStore {
  const details = new Map<string, Detail>();
  const listeners = new Set<() => void>();
  return {
    publish(pageId, detail) {
      if (detail === null) details.delete(pageId);
      else details.set(pageId, detail);
      for (const fn of [...listeners]) fn();
    },
    get(pageId) {
      return details.get(pageId) ?? null;
    },
    subscribe(fn) {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
  };
}

let singleton: PageDetailStore | null = null;

/** Process-wide store. */
export function getPageDetailStore(): PageDetailStore {
  if (!singleton) singleton = createPageDetailStore();
  return singleton;
}
