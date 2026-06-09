// Cross-page "reveal this comment" channel.
//
// The Comments dashboard lists every comment in the stream. Its
// "Go to location" button navigates to the comment's target page
// (file / wiki / task) and then asks that page to scroll to — and open
// — the specific anchored comment. Navigation is async (the target
// surface mounts and fetches its threads after the click), so the
// request can't be delivered as a direct call; instead we stash the
// requested comment id here until the surface that owns it consumes it.
//
// A surface (MonacoCommentLayer, RichTextField) subscribes, and whenever
// its own threads include the pending id it reveals the anchor and calls
// `clearCommentReveal`. The pending id therefore survives the navigation
// gap and a slow threads fetch, and is dropped exactly once.

type Listener = () => void;

let pending: string | null = null;
const listeners = new Set<Listener>();

/// Ask whichever comment surface owns `commentId` to reveal it.
export function requestCommentReveal(commentId: string): void {
  pending = commentId;
  for (const l of listeners) l();
}

/// The comment id awaiting reveal, or null. Surfaces only act on it when
/// it matches one of their own threads.
export function peekPendingCommentReveal(): string | null {
  return pending;
}

/// Drop the pending reveal once a surface has handled it. No-op if a
/// different id is now pending (a newer request superseded this one).
export function clearCommentReveal(commentId: string): void {
  if (pending === commentId) pending = null;
}

/// Subscribe to reveal requests. Returns an unsubscribe fn.
export function subscribeCommentReveal(cb: Listener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
