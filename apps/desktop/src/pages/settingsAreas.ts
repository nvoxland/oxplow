/// Settings' areas: one Document page each (`page:settings#area=<id>`),
/// sharing a rail that lists them. Each area's sections have stable ids —
/// the same ones `SLOTS` lists for its `settings.<area>.sections` slot, so
/// an extension adds its own settings beside the right one.

export type SettingsAreaId = "overview" | "agents" | "lsp" | "extensions" | "data" | "integrations" | "ai";

export interface SettingsArea {
  id: SettingsAreaId;
  /** Its name in the rail and the launcher. */
  title: string;
  /** One line under the page title: what the area is for. */
  intro: string;
  /** Its own sections, in order: stable ids (keep them in step with `SLOTS`). */
  sections: readonly { id: string; title: string }[];
  /** What the area's slot binds, when it binds any (`SLOTS` lists the same). */
  slotParams?: readonly string[];
}

export const SETTINGS_AREAS: readonly SettingsArea[] = [
  {
    id: "overview",
    title: "Every Setting",
    intro: "What shapes this project, where each value comes from, and Ask the Agent to Change This.",
    sections: [],
    slotParams: ["q"],
  },
  {
    id: "agents",
    title: "Agents",
    intro: "The coding agents this project runs, what new threads start with, and what every agent is told.",
    sections: [],
  },
  {
    id: "lsp",
    title: "Language Servers",
    intro: "The language servers oxplow runs for code intelligence. Changes apply at once.",
    sections: [],
  },
  {
    id: "extensions",
    title: "Extensions",
    intro: "Extensions add lenses, pages and data. Their sources are in .oxplow/extensions/; what runs is each one's deployment, with the permissions you approved when it was deployed. Right-click a row to deploy, roll back, enable, share or remove it.",
    sections: [],
  },
  {
    id: "data",
    title: "Data",
    intro: "What oxplow can query, the programs the project would run, and how events reach their reactions.",
    sections: [],
  },
  {
    id: "integrations",
    title: "Integrations",
    intro: "Which implementation each capability uses, and the outside systems extensions connect to.",
    sections: [{ id: "integrations", title: "Integrations" }],
  },
  {
    id: "ai",
    title: "AI",
    intro: "The models oxplow itself calls — for summaries, typed questions and extensions. Your coding agents are separate.",
    sections: [],
  },
];

/** The area named `id`; the overview for anything else. */
export function settingsArea(id: string | null | undefined): SettingsArea {
  return SETTINGS_AREAS.find((a) => a.id === id) ?? SETTINGS_AREAS[0]!;
}

/** The parts slot an area's page offers extensions. */
export function settingsSlot(area: SettingsAreaId): string {
  return `settings.${area}.sections`;
}
