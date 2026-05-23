// Global suppressor for the native WKWebView/macOS context menu.
//
// Tauri's webview renders the OS-default right-click menu (Look Up,
// Translate, Copy, Share, Inspect Element, Services, …) anywhere there
// is selectable content unless the page cancels the `contextmenu`
// event. Oxplow's own per-row menus are visible kebab `⋯` popovers
// (see `.context/usability.md`), so the OS menu is never wanted —
// except where the native menu carries real editing affordances:
// text inputs / textareas, contenteditable surfaces (Tiptap), the
// Monaco editor (which manages its own menu), and the xterm terminal
// (right-click copy/paste). Those are exempted.

/// A minimal, DOM-free description of an element in the target's
/// ancestor chain (closest-first). The listener builds this from the
/// real DOM; the decision logic stays pure so it's unit-testable
/// without a DOM in the bun test environment.
export interface ElementDescriptor {
  /** Lowercased tag name, e.g. "input". */
  tag: string;
  /** classList entries. */
  classes: string[];
  /** The `contentEditable` IDL value: "true" | "false" | "" | "inherit". */
  contentEditable?: string;
}

/// Decide whether to cancel the native context menu for a click whose
/// target has the given ancestor chain (closest element first). Returns
/// true to suppress (the default) and false to let the OS menu through.
export function shouldSuppressContextMenu(chain: ElementDescriptor[]): boolean {
  for (const node of chain) {
    if (node.tag === "input" || node.tag === "textarea") return false;
    if (node.contentEditable === "true" || node.contentEditable === "") return false;
    if (node.classes.includes("monaco-editor")) return false;
    if (node.classes.includes("xterm")) return false;
  }
  return true;
}

/// Build the closest-first ancestor [`ElementDescriptor`] chain for an
/// event target. Exported so the selection-comment affordance can reuse
/// the exact same exemption test (`shouldSuppressContextMenu`) — a
/// region where the OS menu is suppressed is also where our own
/// selection affordances belong, and editors/inputs/terminal are the
/// shared carve-out.
export function describeChain(target: EventTarget | null): ElementDescriptor[] {
  const chain: ElementDescriptor[] = [];
  let el = target instanceof Element ? target : null;
  while (el) {
    chain.push({
      tag: el.tagName.toLowerCase(),
      classes: Array.from(el.classList),
      contentEditable: (el as HTMLElement).contentEditable,
    });
    el = el.parentElement;
  }
  return chain;
}

/// Install the global suppressor. Uses the capture phase so it runs
/// before app-level handlers that might `stopPropagation` (the same
/// reasoning as the Cmd+K palette listener). Returns a cleanup fn.
export function installContextMenuSuppressor(target: Window = window): () => void {
  const handler = (event: Event) => {
    if (shouldSuppressContextMenu(describeChain(event.target))) {
      event.preventDefault();
    }
  };
  target.addEventListener("contextmenu", handler, true);
  return () => target.removeEventListener("contextmenu", handler, true);
}
