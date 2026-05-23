// Cross-surface "open the comment composer" channel.
//
// Most commentable surfaces capture a *text selection* and show the
// floating toolbar. But draggable rows (task list, file tree) can't be
// drag-selected — a mousedown starts a drag. For those, a right-click
// "Comment" menu item builds the same pending-comment payload from the
// row's element + label text and dispatches it here; the single
// app-level `DomCommentLayer` (via `useDomAnnotations`) subscribes and
// opens its composer directly.
//
// One app-level composer owns creation, so this is a simple
// fan-out — every subscriber (there is one) is notified.

import type { PendingComment } from "./components/Comments/useDomAnnotations.js";

type Listener = (req: PendingComment) => void;
const listeners = new Set<Listener>();

/// Ask the app-level comment layer to open its composer for `req`.
export function requestCommentCompose(req: PendingComment): void {
  for (const l of listeners) l(req);
}

/// Subscribe to compose requests. Returns an unsubscribe fn.
export function subscribeCommentCompose(cb: Listener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
