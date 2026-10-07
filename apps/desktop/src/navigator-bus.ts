// Cross-surface "show the navigator" channel.
//
// The Navigator's panel open/close state is its own (useSlideoutStrip).
// Surfaces that point at it — the title bar's stream name — dispatch here;
// the Navigator subscribes and opens its panel. Same fan-out shape as
// new-thread-bus.

type Listener = () => void;
const listeners = new Set<Listener>();

/// Ask the Navigator to open its panel of streams and threads.
export function requestNavigatorOpen(): void {
  for (const l of listeners) l();
}

/// Subscribe to open requests. Returns an unsubscribe fn.
export function subscribeNavigatorOpenRequests(cb: Listener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
