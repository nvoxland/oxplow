/// The prompt catalog as pages use it: what to ask about a kind of
/// ref, grouped for the catalog page, and the text an Ask inserts.
import { formatContextMention } from "../../agent-context-ref.js";
import type { CatalogPrompt } from "../../tauri-bridge/generated/bindings.js";

/** The prompts a page for a ref of `kind` suggests. */
export function promptsAbout(catalog: CatalogPrompt[], kind: string): CatalogPrompt[] {
  return catalog.filter((p) => p.about === kind);
}

export interface PromptGroup {
  /** The area or extension name. */
  name: string;
  /** How a person reads it: an area's label, an extension's name. */
  label: string;
  kind: "area" | "extension";
  prompts: CatalogPrompt[];
}

/** One of core's areas as a person reads it (not `code_intel`,
 *  `work_items`). */
const AREA_LABELS: Record<string, string> = {
  code_intel: "Code",
  knowledge: "Wiki",
  extensions: "Extensions",
  vcs: "Version control",
  work_items: "Work items",
};

export function areaLabel(name: string): string {
  return AREA_LABELS[name] ?? name.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
}

/** Grouped by who offers them: core's areas, then extensions, each by name. */
export function promptsBySource(catalog: CatalogPrompt[]): PromptGroup[] {
  const groups = new Map<string, PromptGroup>();
  for (const p of catalog) {
    const key = `${p.source.kind}:${p.source.name}`;
    const label = p.source.kind === "area" ? areaLabel(p.source.name) : p.source.name;
    const group = groups.get(key) ?? { name: p.source.name, label, kind: p.source.kind, prompts: [] };
    group.prompts.push(p);
    groups.set(key, group);
  }
  const order = (g: PromptGroup) => (g.kind === "area" ? 0 : 1);
  return [...groups.values()].sort((a, b) => order(a) - order(b) || a.label.localeCompare(b.label));
}

/** What Ask puts in the agent's input: the ref it's about (if any), then
 *  the question. Never sent — the person edits and sends it. */
export function askText(prompt: string, ref: string | null): string {
  return ref ? `${formatContextMention({ kind: "ref", ref })}${prompt}` : prompt;
}
