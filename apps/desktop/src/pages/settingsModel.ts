/// Settings as a view (P6.H1): the effective settings (`effective_config`)
/// grouped, searched, and turned into an Ask for the agent — the person
/// changes a setting by asking; person-only ones keep a direct control.
import type { EffectiveSetting } from "../tauri-bridge/generated/bindings.js";

export interface SettingsGroup {
  title: string;
  settings: EffectiveSetting[];
}

const GROUPS: { title: string; matches(key: string): boolean }[] = [
  { title: "Project", matches: (k) => ["projectName", "zones", "iconTint"].includes(k) },
  {
    title: "Agents",
    matches: (k) => ["agents", "agentConfig", "acpAgents", "agentPromptAppend", "injectSessionContext"].includes(k),
  },
  { title: "AI", matches: (k) => k === "ai" || k.startsWith("ai.") },
  {
    title: "Snapshots",
    matches: (k) => k.startsWith("snapshot") || k === "generated" || k === "symbolsMaxFilesPerSnapshot",
  },
  { title: "Testing", matches: (k) => k === "testing" },
  { title: "Language Servers", matches: (k) => k === "lsp" },
  { title: "Extensions", matches: (k) => k === "extensions" || k === "extensionInstances" },
  {
    title: "Metrics & Data",
    matches: (k) =>
      k.startsWith("metric") || k.startsWith("dimension") || ["collectors", "measures"].includes(k),
  },
];

/** Settings by group, in `GROUPS` order; anything unmatched under Other. */
export function groupSettings(settings: EffectiveSetting[]): SettingsGroup[] {
  const groups = GROUPS.map((g) => ({ title: g.title, settings: settings.filter((s) => g.matches(s.key)) }));
  const other = settings.filter((s) => !GROUPS.some((g) => g.matches(s.key)));
  return [...groups, { title: "Other", settings: other }].filter((g) => g.settings.length > 0);
}

/** The value, compact, for a row. */
export function valueText(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value === "" ? '""' : value;
  const text = JSON.stringify(value);
  return text.length > 80 ? `${text.slice(0, 77)}…` : text;
}

/** Whether a row matches a search: its key, its doc, or anywhere in its
 *  whole value (not `valueText`'s truncated display). */
export function matchesSearch(s: EffectiveSetting, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const value = typeof s.value === "string" ? s.value : JSON.stringify(s.value ?? null);
  return [s.key, s.doc, value].some((t) => t.toLowerCase().includes(q));
}

/** The prompt Ask the Agent to Change This puts in the agent's input:
 *  which setting, what it's for, what it is now — the agent changes it
 *  with `oxplow.config.set` (and a person-only one asks the person first). */
export function askToChange(s: EffectiveSetting): string {
  const now = `${valueText(s.value)}${s.origin === "default" ? " (the default)" : ""}`;
  const from = s.origin === "extension" && s.extension ? ` from the \`${s.extension}\` extension` : "";
  // Project keys are `.oxplow/project.yaml`'s (oxplow.config.set); scoped ones
  // (`ai.roles.*`, `metrics.*`) live where their origin says.
  const how = s.key.includes(".")
    ? `(It comes from ${s.origin === "default" ? "the defaults" : `the ${s.origin} config`}; tell me how you'd change it.)`
    : "(Use oxplow.config.set; tell me what you changed.)";
  return `Change the setting \`${s.key}\`${from} — ${s.doc || "no description"} It is now ${now}. Change it to: \n${how}`;
}
