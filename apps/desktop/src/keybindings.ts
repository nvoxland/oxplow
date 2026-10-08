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

/** The offer `event` runs (`commandOffers`): of those whose `ui.shortcut`
 *  it is and whose `when` holds now — while the person types in a field,
 *  only those that run while typing — oxplow's own first, so an
 *  extension's command on the same key runs only where oxplow's doesn't
 *  (VS Code's: one key, told apart by `when`). */
export function offerForShortcut(offers: CommandEntry[], event: KeyPress, typing: boolean): CommandEntry | null {
  const runs = offers.filter(
    (o) => o.shortcut && o.enabled !== false && matchesShortcut(event, o.shortcut) && (!typing || o.whileTyping),
  );
  return runs.find((o) => o.id.startsWith("oxplow.")) ?? runs[0] ?? null;
}
