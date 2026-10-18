/// The prompt catalog (`v_prompt`) as pages use it: what to ask about a
/// kind of ref, and the text an Ask inserts.
import { formatContextMention } from "../../agent-context-ref.js";

/** One question a person can ask, with who offers it. */
export interface CatalogPrompt {
  prompt: string;
  /** A ref kind (`file`, `commit`, `effort`): a page for one offers it. */
  about: string | null;
  source: { kind: "area" | "extension"; name: string };
}

/** The prompts a page for a ref of `kind` suggests. */
export function promptsAbout(catalog: CatalogPrompt[], kind: string): CatalogPrompt[] {
  return catalog.filter((p) => p.about === kind);
}

/** What Ask puts in the agent's input: the ref it's about (if any), then
 *  the question. Never sent — the person edits and sends it. */
export function askText(prompt: string, ref: string | null): string {
  return ref ? `${formatContextMention({ kind: "ref", ref })}${prompt}` : prompt;
}
