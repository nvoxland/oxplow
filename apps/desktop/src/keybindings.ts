import type { CommandEntry } from "./components/quickOpenResults.js";

/** A key press, as a shortcut sees it. */
export interface KeyPress {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

/** Whether `event` is `shortcut` (`Ctrl/Cmd+S`, `Ctrl/Cmd+Shift+N`;
 *  `Ctrl/Cmd` is either). */
export function matchesShortcut(event: KeyPress, shortcut: string): boolean {
  const parts = shortcut.split("+");
  const key = parts.pop()?.toLowerCase();
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  const mod = mods.has("ctrl/cmd");
  return (
    event.key.toLowerCase() === key &&
    (event.metaKey || event.ctrlKey) === mod &&
    event.shiftKey === mods.has("shift") &&
    event.altKey === mods.has("alt")
  );
}

/** The offer whose `ui.shortcut` `event` is (`commandOffers`) — while the
 *  person types in a field, only one that runs while typing. */
export function offerForShortcut(offers: CommandEntry[], event: KeyPress, typing: boolean): CommandEntry | null {
  return (
    offers.find((o) => o.shortcut && matchesShortcut(event, o.shortcut) && (!typing || o.whileTyping)) ?? null
  );
}
