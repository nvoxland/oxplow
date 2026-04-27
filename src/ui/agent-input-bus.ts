/**
 * Renderer-side bus for "insert text into the active agent terminal".
 *
 * Drag-drop and right-click "Add to agent context" handlers anywhere in
 * the UI publish text via `insertIntoAgent`; the visible TerminalPane
 * subscribes while mounted and writes the text into the agent's stdin
 * (`term.paste` for direct mode, `sendTerminalMessage` for tmux).
 *
 * Multiple TerminalPanes may exist (one per stream/thread) but only the
 * one whose `visible` prop is true subscribes — so a publish naturally
 * targets the agent the user is currently looking at. If no pane is
 * visible, publish is a no-op (gesture handlers gate on a selected
 * thread anyway).
 */

type Listener = (text: string) => void;

const listeners = new Set<Listener>();

export function subscribeAgentInput(fn: Listener): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

export function insertIntoAgent(text: string): void {
  // Snapshot before iterating so a listener that unsubscribes itself
  // during the call doesn't skip subsequent listeners.
  for (const listener of [...listeners]) {
    try {
      listener(text);
    } catch {
      // Swallow — a single bad subscriber must not prevent the rest
      // from receiving the text.
    }
  }
}
