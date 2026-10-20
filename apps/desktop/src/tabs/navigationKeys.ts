/**
 * The keys that move through a tab's history and origin, as a browser's
 * do: Cmd+[ and Cmd+] for Back and Forward, Alt+ArrowUp and Alt+ArrowDown
 * for Up and Down the collection the page was opened from. They are the
 * global handler's own (not commands): typing in a field, a select or a
 * contenteditable, or working in Monaco, keeps them.
 */
export type NavKey = "back" | "forward" | "up" | "down";

interface KeyLike {
  key: string;
  metaKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}

/** Whether focus is somewhere the keys are the field's. */
export function typingIn(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT" || tag === "INPUT") return true;
  return target.closest(".monaco-editor") !== null;
}

export function navigationKey(event: KeyLike, target: EventTarget | null): NavKey | null {
  if (typingIn(target)) return null;
  if (event.metaKey && !event.altKey && !event.ctrlKey && !event.shiftKey) {
    if (event.key === "[") return "back";
    if (event.key === "]") return "forward";
  }
  if (event.altKey && !event.metaKey && !event.ctrlKey && !event.shiftKey) {
    if (event.key === "ArrowUp") return "up";
    if (event.key === "ArrowDown") return "down";
  }
  return null;
}
