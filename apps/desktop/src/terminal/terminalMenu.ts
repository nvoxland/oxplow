/// The terminal's right-click menu, oxplow's own in place of the OS's
/// (`context-menu.ts` no longer exempts xterm): Copy and Add Selection to
/// Context act on the selection, Paste writes the clipboard through
/// `term.paste`, and with nothing selected Add to Context puts the
/// terminal's own page (the session) in the basket. The same entry, the
/// same @, as everywhere else (`.context/usability.md` "Add to Context").
/// A program that has the mouse (Claude Code) draws its own selection, which
/// the terminal can't read: with nothing selected the menu says how to make
/// the terminal's own (Option+drag on macOS, `macOptionClickForcesSelection`;
/// Shift+drag elsewhere, xterm's own) and holds only Paste: Copy and Add
/// Selection to Context come with that selection. Over a link (a path or a
/// web URL) the link's own items come first: Open In ▸ and its Copy.
import type { MenuItem } from "../menu.js";

export interface TerminalMenuSources {
  /** The selected text; empty when none. */
  selection: string;
  /** The lines around it (`terminalSelection.ts`). */
  around?: string;
  /** Whether the program in the terminal has the mouse. */
  mouseCaptured?: boolean;
  /** macOS: the hint names Option, not Shift. */
  mac?: boolean;
  /** The terminal's URI: what Add to Context names without a selection. */
  uri: string;
  /** The items of the link under the pointer (Open In ▸, Copy): first. */
  link?: MenuItem[];
  copy(text: string): void;
  paste(): void;
  add(what: { uri: string; text?: string; around?: string }): void;
}

export function terminalMenuItems(sources: TerminalMenuSources): MenuItem[] {
  const own = ownItems(sources);
  const link = sources.link ?? [];
  return link.length > 0 ? [...link, { id: "link-separator", label: "", enabled: false, separator: true }, ...own] : own;
}

function ownItems({ selection, around, mouseCaptured, mac, uri, copy, paste, add }: TerminalMenuSources): MenuItem[] {
  const selected = selection.length > 0;
  if (mouseCaptured && !selected) {
    return [
      { id: "select-hint", label: `Hold ${mac ? "⌥" : "Shift"} and drag to select text to copy or add to context`, enabled: false },
      { id: "paste", label: "Paste", enabled: true, run: paste },
    ];
  }
  return [
    { id: "copy", label: "Copy", enabled: selected, run: () => copy(selection) },
    { id: "paste", label: "Paste", enabled: true, run: paste },
    {
      id: "add-to-context",
      label: selected ? "Add Selection to Context" : "Add to Context",
      enabled: true,
      icon: "at-sign",
      run: () => add(selected ? { uri, text: selection, ...(around ? { around } : {}) } : { uri }),
    },
  ];
}
