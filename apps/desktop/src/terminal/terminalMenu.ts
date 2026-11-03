/// The terminal's right-click menu, oxplow's own in place of the OS's
/// (`context-menu.ts` no longer exempts xterm): Copy and Add Selection to
/// Context act on the selection, Paste writes the clipboard through
/// `term.paste`, and with nothing selected Add to Context puts the
/// terminal's own page (the session) in the basket. The same entry, the
/// same @, as everywhere else (`.context/usability.md` "Add to Context").
import type { MenuItem } from "../menu.js";

export interface TerminalMenuSources {
  /** The selected text; empty when none. */
  selection: string;
  /** The terminal's URI: what Add to Context names without a selection. */
  uri: string;
  copy(text: string): void;
  paste(): void;
  add(what: { uri: string; text?: string }): void;
}

export function terminalMenuItems({ selection, uri, copy, paste, add }: TerminalMenuSources): MenuItem[] {
  const selected = selection.length > 0;
  return [
    { id: "copy", label: "Copy", enabled: selected, run: () => copy(selection) },
    { id: "paste", label: "Paste", enabled: true, run: paste },
    {
      id: "add-to-context",
      label: selected ? "Add Selection to Context" : "Add to Context",
      enabled: true,
      icon: "at-sign",
      run: () => add(selected ? { uri, text: selection } : { uri }),
    },
  ];
}
