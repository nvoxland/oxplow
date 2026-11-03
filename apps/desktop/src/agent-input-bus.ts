/**
 * Renderer-side bus for "insert text into the active agent terminal".
 *
 * The `agent_input.write` command (a page action's draft) and Repair with
 * the Agent publish text via `insertIntoAgent`; the visible TerminalSurface
 * subscribes while mounted and writes the text into the agent's stdin
 * (`term.paste`). What the person wants an agent to read goes in the
 * thread's context basket instead (`contextBasket.ts`).
 * This is still HUMAN input — a person's gesture is the only thing that
 * publishes here; oxplow never synthesizes agent input on its own (see
 * .context/agent-model.md).
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

/**
 * Inserting fills the agent's input and never sends it. The terminal
 * pastes what arrives here, and xterm turns every line break into Enter —
 * so this is the one place line breaks become spaces, whatever the
 * caller handed it (a manifest's prompt, a selection, a row's text).
 */
export function oneLine(text: string): string {
  return text.replace(/\r\n|\r|\n/g, " ");
}

export function insertIntoAgent(text: string): void {
  const line = oneLine(text);
  // Snapshot before iterating so a listener that unsubscribes itself
  // during the call doesn't skip subsequent listeners.
  for (const listener of [...listeners]) {
    try {
      listener(line);
    } catch {
      // Swallow — a single bad subscriber must not prevent the rest
      // from receiving the text.
    }
  }
}
