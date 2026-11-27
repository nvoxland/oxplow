/// The keys an on-screen keyboard lacks, as the bytes a hardware keyboard
/// sends: an agent's prompts need Esc, Tab, Shift+Tab, Ctrl and the arrows
/// (Claude's "trust this folder" is answered with the arrows). The touch bar
/// under a terminal draws them (`TerminalTouchBar`).

export type TouchKeyId = "esc" | "backtab" | "tab" | "ctrl" | "left" | "up" | "down" | "right";

/** The bar's keys, in the order drawn. `ctrl` is a modifier, not a key. */
export const TOUCH_KEYS: readonly { id: TouchKeyId; label: string; name: string }[] = [
  { id: "esc", label: "Esc", name: "Escape" },
  { id: "backtab", label: "⇧Tab", name: "Shift+Tab" },
  { id: "tab", label: "Tab", name: "Tab" },
  { id: "ctrl", label: "Ctrl", name: "Ctrl (applies to the next key typed)" },
  { id: "left", label: "←", name: "Left arrow" },
  { id: "up", label: "↑", name: "Up arrow" },
  { id: "down", label: "↓", name: "Down arrow" },
  { id: "right", label: "→", name: "Right arrow" },
];

const ARROW: Record<"up" | "down" | "right" | "left", string> = { up: "A", down: "B", right: "C", left: "D" };

/** The bytes key `id` sends. Arrows follow the program's cursor-key mode
 *  (DECCKM, xterm's `modes.applicationCursorKeysMode`): full-screen
 *  programs and menus turn it on and read `ESC O A`, a shell reads `ESC [ A`. */
export function keyBytes(id: Exclude<TouchKeyId, "ctrl">, applicationCursor: boolean): string {
  switch (id) {
    case "esc":
      return "\x1b";
    case "tab":
      return "\t";
    case "backtab":
      return "\x1b[Z";
    default:
      return `\x1b${applicationCursor ? "O" : "["}${ARROW[id]}`;
  }
}

/** `typed` with Ctrl held: a letter or one of `@[\]^_ ` becomes its control
 *  character. Null for anything else (a word a keyboard's suggestion typed
 *  at once, a digit), which then goes as it is. */
export function ctrlOf(typed: string): string | null {
  if (typed.length !== 1) return null;
  if (typed === " ") return "\x00";
  const code = typed.toUpperCase().charCodeAt(0);
  if (code >= 0x40 && code <= 0x5f) return String.fromCharCode(code - 0x40);
  return null;
}
