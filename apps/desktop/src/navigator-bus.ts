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

/// A stream's or thread's right-click menu, asked for from outside the
/// Navigator (the title bar's names), opened at `x`/`y`. The Navigator owns
/// the menus — they act on its panel (rename, add thread) — so it opens
/// them; the asker names what and where.
export interface NavigatorMenuRequest {
  kind: "stream" | "thread";
  id: string;
  x: number;
  y: number;
}

type MenuListener = (request: NavigatorMenuRequest) => void;
const menuListeners = new Set<MenuListener>();

/// Ask the Navigator to open the menu for a stream or thread.
export function requestNavigatorMenu(request: NavigatorMenuRequest): void {
  for (const l of menuListeners) l(request);
}

/// Subscribe to menu requests. Returns an unsubscribe fn.
export function subscribeNavigatorMenuRequests(cb: MenuListener): () => void {
  menuListeners.add(cb);
  return () => {
    menuListeners.delete(cb);
  };
}
