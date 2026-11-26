/// What a mouse shows on hover (a control's `title`), shown by a finger: press
/// and hold an icon-only control (it has a `title` or `aria-label` and no
/// text of its own) and its label shows in a bubble above it; the press then
/// does nothing more (the click that follows is cancelled). Touch presses
/// only, so a mouse never sees it. One listener for the whole page, mounted
/// once (`main.tsx`): no control needs anything of its own.

/** How long a finger holds before the label shows. Android's own long press
 *  (its `contextmenu`) may come before or after, by the device's setting:
 *  nothing here depends on the order. */
export const HOLD_MS = 400;
/** How long after a hold its click is taken as the hold's own. */
const SWALLOW_MS = 800;
/** How far a finger may move and still be holding, not scrolling. */
const SLOP_PX = 10;

/** The label a finger's hold on `el` shows: the nearest control's `title`, or
 *  its `aria-label` when it has no text to read. `null`: nothing to show
 *  (a control with words says what it does). Pure over the element;
 *  exported for tests. */
export function hintFor(el: Element | null): string | null {
  const control = el?.closest("button, a, [role=button], [title]");
  if (!control) return null;
  const raw = (control.textContent ?? "").trim();
  // A glyph (×, ☰, ▾) is no word: it says nothing a finger can read.
  const text = /[\p{L}\p{N}]/u.test(raw) ? raw : "";
  const title = control.getAttribute("title")?.trim();
  if (title && title !== text) return title;
  // Its own text, cut short (a tab's name, a nav title): shown whole.
  const box = control as HTMLElement;
  if (title && box.scrollWidth > box.clientWidth) return title;
  const label = control.getAttribute("aria-label")?.trim();
  if (label && text === "") return label;
  return null;
}

export function installTouchHints(doc: Document = document): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start: { x: number; y: number } | null = null;
  let bubble: HTMLElement | null = null;
  let swallowClick = false;
  let swallowTimer: ReturnType<typeof setTimeout> | null = null;
  const endSwallow = () => {
    swallowClick = false;
    if (swallowTimer !== null) clearTimeout(swallowTimer);
    swallowTimer = null;
  };

  const hide = () => {
    bubble?.remove();
    bubble = null;
  };
  const cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    start = null;
  };
  const show = (target: Element, text: string) => {
    hide();
    const rect = target.getBoundingClientRect();
    const view = doc.defaultView;
    bubble = doc.createElement("div");
    bubble.setAttribute("role", "tooltip");
    bubble.setAttribute("data-testid", "touch-hint");
    bubble.textContent = text;
    // Above the control, or below it near the top of the screen.
    const below = rect.top < 48;
    Object.assign(bubble.style, {
      position: "fixed",
      left: "0px",
      top: below ? `${rect.bottom + 8}px` : `${rect.top - 8}px`,
      transform: below ? "none" : "translateY(-100%)",
      maxWidth: "70vw",
      padding: "6px 10px",
      borderRadius: "var(--radius-md)",
      background: "var(--surface-elevated)",
      color: "var(--text-primary)",
      border: "1px solid var(--border-strong)",
      fontSize: "var(--text-sm)",
      zIndex: "100000",
      pointerEvents: "none",
    });
    doc.body.appendChild(bubble);
    // Centred on the control, kept inside the screen's sides.
    const width = bubble.getBoundingClientRect().width;
    const screen = view?.innerWidth ?? width + 16;
    const left = Math.min(Math.max(rect.left + rect.width / 2 - width / 2, 8), Math.max(8, screen - width - 8));
    bubble.style.left = `${left}px`;
  };

  const onDown = (e: PointerEvent) => {
    // A new press: whatever a hold before it waited to swallow is over
    // (Chrome may send no click after a long press).
    endSwallow();
    hide();
    if (e.pointerType !== "touch") return;
    const target = e.target instanceof Element ? e.target : null;
    const text = hintFor(target);
    if (!target || text === null) return;
    start = { x: e.clientX, y: e.clientY };
    timer = setTimeout(() => {
      timer = null;
      show(target, text);
      swallowClick = true;
      swallowTimer = setTimeout(endSwallow, SWALLOW_MS);
    }, HOLD_MS);
  };
  const onMove = (e: PointerEvent) => {
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) cancel();
  };
  const onUp = () => cancel();
  // The hold showed the label: the press was a question, not a press.
  const onClick = (e: MouseEvent) => {
    if (!swallowClick) return;
    endSwallow();
    e.preventDefault();
    e.stopPropagation();
  };
  // The long press opened a menu of the app's: the menu is the answer, so
  // the bubble goes and a tap on its items is a tap.
  const onContextMenu = () => {
    hide();
    endSwallow();
  };
  const onDrag = () => {
    cancel();
    hide();
    endSwallow();
  };
  const onScroll = () => {
    cancel();
    hide();
  };

  doc.addEventListener("pointerdown", onDown, true);
  doc.addEventListener("pointermove", onMove, true);
  doc.addEventListener("pointerup", onUp, true);
  doc.addEventListener("pointercancel", onUp, true);
  doc.addEventListener("click", onClick, true);
  doc.addEventListener("contextmenu", onContextMenu, true);
  doc.addEventListener("scroll", onScroll, true);
  doc.addEventListener("dragstart", onDrag, true);
  return () => {
    cancel();
    hide();
    endSwallow();
    doc.removeEventListener("dragstart", onDrag, true);
    doc.removeEventListener("pointerdown", onDown, true);
    doc.removeEventListener("pointermove", onMove, true);
    doc.removeEventListener("pointerup", onUp, true);
    doc.removeEventListener("pointercancel", onUp, true);
    doc.removeEventListener("click", onClick, true);
    doc.removeEventListener("contextmenu", onContextMenu, true);
    doc.removeEventListener("scroll", onScroll, true);
  };
}
