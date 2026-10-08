/// The prompt catalog as pages use it (P6.D2): what to ask about a kind of
/// ref, grouped for the catalog page, and the text an Ask inserts.
import { formatContextMention } from "../../agent-context-ref.js";
import type { CatalogPrompt } from "../../tauri-bridge/generated/bindings.js";

/** The prompts a page for a ref of `kind` suggests. */
export function promptsAbout(catalog: CatalogPrompt[], kind: string): CatalogPrompt[] {
  return catalog.filter((p) => p.about === kind);
}

export interface PromptGroup {
  /** The capability or extension name. */
  name: string;
  /** How a person reads it: a capability's label, an extension's name. */
  label: string;
  kind: "capability" | "extension";
  prompts: CatalogPrompt[];
}

/** Grouped by who offers them: capabilities, then extensions, each by name. */
/** A core capability's name as a person reads it (tsk1044: the headings
 *  showed `code_intel`, `work_items`). */
const CAPABILITY_LABELS: Record<string, string> = {
  code_intel: "Code",
  knowledge: "Wiki",
  extensions: "Extensions",
  vcs: "Version control",
  work_items: "Work items",
};

export function capabilityLabel(name: string): string {
  return CAPABILITY_LABELS[name] ?? name.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
}

export function promptsBySource(catalog: CatalogPrompt[]): PromptGroup[] {
  const groups = new Map<string, PromptGroup>();
  for (const p of catalog) {
    const key = `${p.source.kind}:${p.source.name}`;
    const label = p.source.kind === "capability" ? capabilityLabel(p.source.name) : p.source.name;
    const group = groups.get(key) ?? { name: p.source.name, label, kind: p.source.kind, prompts: [] };
    group.prompts.push(p);
    groups.set(key, group);
  }
  const order = (g: PromptGroup) => (g.kind === "capability" ? 0 : 1);
  return [...groups.values()].sort((a, b) => order(a) - order(b) || a.label.localeCompare(b.label));
}

/** What Ask puts in the agent's input: the ref it's about (if any), then
 *  the question. Never sent — the person edits and sends it. */
export function askText(prompt: string, ref: string | null): string {
  return ref ? `${formatContextMention({ kind: "ref", ref })}${prompt}` : prompt;
}
