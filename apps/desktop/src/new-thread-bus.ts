// Cross-surface "start a new thread" channel.
//
// Thread creation is a flow of the window's stream and thread menus
// (`window/useStreamThreadMenus.tsx`), drawn by the shell (the desktop's
// Navigator panel). Surfaces that can't reach it directly — the command
// palette's "New Thread…", keyboard shortcuts — dispatch here; the hook
// subscribes, reveals the shell's place for it and starts the creator for
// the requested stream.

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
