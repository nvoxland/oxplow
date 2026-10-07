/// Settings' sections, and going to one (tsk1040): an alert or a link lands
/// on its section, whether Settings is open already (it hears the request)
/// or opens now (it takes the pending one when it mounts).

export interface SettingsSection {
  id: string;
  title: string;
}

/** Every section, in page order — the page's index. */
export const SETTINGS_SECTIONS: SettingsSection[] = [
  { id: "settings-every", title: "Every Setting" },
  { id: "settings-agents", title: "Agents" },
  { id: "settings-prompt", title: "Agent Prompt" },
  { id: "settings-lsp", title: "Language Servers" },
  { id: "settings-extensions", title: "Extensions" },
  { id: "settings-data", title: "Data" },
  { id: "settings-data-programs", title: "Programs" },
  { id: "settings-data-delivery", title: "Delivery" },
  { id: "settings-pieces", title: "Pieces" },
  { id: "settings-integrations", title: "Integrations" },
  { id: "settings-ai", title: "AI" },
];

let pending: string | null = null;
const listeners = new Set<(id: string) => void>();

/** Go to section `id`: an open Settings scrolls there; otherwise the next
 *  one to open does. */
export function goToSettingsSection(id: string): void {
  if (listeners.size > 0) {
    for (const l of listeners) l(id);
    pending = null;
    return;
  }
  pending = id;
}

/** The section asked for before Settings opened, once. */
export function takeSettingsSection(): string | null {
  const id = pending;
  pending = null;
  return id;
}

/** Hear each request while Settings is open. */
export function onSettingsSection(listener: (id: string) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Scroll section `id` into view, if it's on the page. */
export function scrollToSettingsSection(id: string): void {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
}
