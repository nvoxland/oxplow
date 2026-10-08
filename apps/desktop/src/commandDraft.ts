/// The draft a command puts in the agent's input (`agent_input.write`
/// `draft`, its `ui.input.text`): so a page that offers the same prompt
/// reads it from the command's one declaration (oxplow-foundation) rather
/// than keeping a copy.
import { usePersonCommands } from "./personCommandsStore.js";

/** Command `id`'s draft text; `null` until the listing has it. */
export function useCommandDraft(id: string): string | null {
  const specs = usePersonCommands();
  const text = (specs.find((s) => s.id === id)?.ui?.input as { text?: unknown } | null | undefined)?.text;
  return typeof text === "string" ? text : null;
}
