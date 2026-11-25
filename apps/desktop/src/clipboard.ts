/** Copy `text`. `navigator.clipboard` is missing outside a secure context
 *  (a page on plain HTTP), so fall back to a hidden textarea and
 *  `execCommand("copy")`. Resolves false when nothing could copy. */
export async function copyText(text: string): Promise<boolean> {
  const api = globalThis.navigator?.clipboard;
  if (api && typeof api.writeText === "function") {
    try { await api.writeText(text); return true; } catch { return false; }
  }
  const doc = globalThis.document;
  if (!doc?.body || typeof doc.execCommand !== "function") return false;
  const ta = doc.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  // The copy needs the text selected in a focused field; give the focus
  // back after, so a copy from the terminal or editor leaves it there.
  const before = doc.activeElement as HTMLElement | null;
  doc.body.appendChild(ta);
  try {
    ta.focus();
    ta.select();
    return doc.execCommand("copy");
  } catch {
    return false;
  } finally {
    ta.remove();
    before?.focus?.();
  }
}

/** Read the clipboard, or null when the page may not (plain HTTP, or
 *  denied): the caller then asks the person to paste with the keyboard. */
export async function readText(): Promise<string | null> {
  const api = globalThis.navigator?.clipboard;
  if (!api || typeof api.readText !== "function") return null;
  try { return await api.readText(); } catch { return null; }
}

export const PASTE_WITH_KEYBOARD = "Paste with the keyboard (Ctrl+V or long-press)";
