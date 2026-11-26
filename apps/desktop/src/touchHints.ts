/// What a mouse shows on hover (a control's `title`), shown by a finger: press
/// and hold an icon-only control (it has a `title` or `aria-label` and no
/// text of its own) and its label shows in a bubble above it; the press then
/// does nothing more (the click that follows is cancelled). Touch presses
/// only, so a mouse never sees it. One listener for the whole page, mounted
/// once (`main.tsx`): no control needs anything of its own.

/** How long a finger holds before the label shows. Chrome on Android sends
 *  its long-press `contextmenu` at about 500 ms, so this comes first. */
export const HOLD_MS = 400;
/** How far a finger may move and still be holding, not scrolling. */
const SLOP_PX = 10;

/** The label a finger's hold on `el` shows: the nearest control's `title`, or
 *  its `aria-label` when it has no text to read. `null`: nothing to show
 *  (a control with words says what it does). Pure over the element;
 *  exported for tests. */
export function hintFor(el: Element | null): string | null {
  const control = el?.closest("button, a, [role=button], [title]");
  if (!control) return null;
  const text = (control.textContent ?? "").trim();
  const title = control.getAttribute("title")?.trim();
  if (title && title !== text) return title;
  const label = control.getAttribute("aria-label")?.trim();
  if (label && text === "") return label;
  return null;
}

export function installTouchHints(doc: Document = document): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start: { x: number; y: number } | null = null;
  let bubble: HTMLElement | null = null;
  let swallowClick = false;

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
    bubble = doc.createElement("div");
    bubble.setAttribute("role", "tooltip");
    bubble.setAttribute("data-testid", "touch-hint");
    bubble.textContent = text;
    Object.assign(bubble.style, {
      position: "fixed",
      left: `${Math.max(8, rect.left + rect.width / 2)}px`,
      top: `${Math.max(8, rect.top - 8)}px`,
      transform: "translate(-50%, -100%)",
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
  };

  const onDown = (e: PointerEvent) => {
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
    }, HOLD_MS);
  };
  const onMove = (e: PointerEvent) => {
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) cancel();
  };
  const onUp = () => cancel();
  // The hold showed the label: the press was a question, not a press.
  const onClick = (e: MouseEvent) => {
    if (!swallowClick) return;
    swallowClick = false;
    e.preventDefault();
    e.stopPropagation();
  };
  const onContextMenu = (e: Event) => {
    if (bubble) e.preventDefault();
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
  return () => {
    cancel();
    hide();
    doc.removeEventListener("pointerdown", onDown, true);
    doc.removeEventListener("pointermove", onMove, true);
    doc.removeEventListener("pointerup", onUp, true);
    doc.removeEventListener("pointercancel", onUp, true);
    doc.removeEventListener("click", onClick, true);
    doc.removeEventListener("contextmenu", onContextMenu, true);
    doc.removeEventListener("scroll", onScroll, true);
  };
}
