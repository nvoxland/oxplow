// Cross-surface "start a new thread" channel.
//
// Thread creation lives in the Navigator's inline flow (title input +
// agent picker). Surfaces that can't reach it directly — the command
// palette's "New Thread…", keyboard shortcuts — dispatch here; the
// Navigator subscribes, opens its overlay, and shows the inline
// creator for the requested stream. Same fan-out shape as
// comment-compose-bus.

type Listener = (streamId: string) => void;
const listeners = new Set<Listener>();

/// Ask the Navigator to open its inline new-thread flow for `streamId`.
export function requestNewThread(streamId: string): void {
  for (const l of listeners) l(streamId);
}

/// Subscribe to new-thread requests. Returns an unsubscribe fn.
export function subscribeNewThreadRequests(cb: Listener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
