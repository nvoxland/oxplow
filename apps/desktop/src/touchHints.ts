/// A finger's hold, for the whole page (one set of listeners, mounted once in
/// `main.tsx`; touch presses only, so a mouse never sees it):
///
/// - What a mouse shows on hover (a control's `title`): hold an icon-only
///   control and its label shows in a bubble above it.
/// - A right-click: a long press is a `contextmenu` at the finger, so every
///   menu of the page opens by touch. Android sends its own (then this sends
///   none, and stops a late one of the browser's: one menu per press); iPad
///   sends none. Text fields, rich text and the code editor keep the OS's
///   own long press (select, paste).
///
/// Either way the press then does nothing more (its click is cancelled). No
/// control needs anything of its own.

import { describeChain, shouldSuppressContextMenu } from "./context-menu.js";

/** How long a finger holds before the label shows. Android's own long press
 *  (its `contextmenu`) may come before or after, by the device's setting:
 *  nothing here depends on the order. */
export const HOLD_MS = 400;
/** How long a finger holds before it is a right-click. */
export const MENU_MS = 550;
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
  let menuTimer: ReturnType<typeof setTimeout> | null = null;
  let start: { x: number; y: number } | null = null;
  // This press has had its menu (the browser's or ours): no second one.
  let menuSent = false;
  let sending = false;
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
    if (menuTimer !== null) clearTimeout(menuTimer);
    timer = null;
    menuTimer = null;
    start = null;
  };
  const swallowNextClick = () => {
    swallowClick = true;
    if (swallowTimer !== null) clearTimeout(swallowTimer);
    swallowTimer = setTimeout(endSwallow, SWALLOW_MS);
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
    cancel();
    menuSent = false;
    if (e.pointerType !== "touch") return;
    const target = e.target instanceof Element ? e.target : null;
    if (!target) return;
    const at = { x: e.clientX, y: e.clientY };
    start = at;
    const text = hintFor(target);
    if (text !== null) {
      timer = setTimeout(() => {
        timer = null;
        show(target, text);
        swallowNextClick();
      }, HOLD_MS);
    }
    // Where the OS's own long press is the right one (text), none of ours.
    if (!shouldSuppressContextMenu(describeChain(target))) return;
    menuTimer = setTimeout(() => {
      menuTimer = null;
      if (menuSent || !target.isConnected) return;
      menuSent = true;
      sending = true;
      try {
        target.dispatchEvent(
          new MouseEvent("contextmenu", { bubbles: true, cancelable: true, composed: true, button: 2, clientX: at.x, clientY: at.y }),
        );
      } finally {
        sending = false;
      }
      swallowNextClick();
      afterMenu();
    }, MENU_MS);
  };
  const onMove = (e: PointerEvent) => {
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) cancel();
  };
  // A lifted finger ends the hold; `start` stays until the next press so a
  // browser menu sent at the lift is still this press's.
  const onUp = () => {
    if (timer !== null) clearTimeout(timer);
    if (menuTimer !== null) clearTimeout(menuTimer);
    timer = null;
    menuTimer = null;
  };
  // The hold showed the label: the press was a question, not a press.
  const onClick = (e: MouseEvent) => {
    if (!swallowClick) return;
    endSwallow();
    e.preventDefault();
    e.stopPropagation();
  };
  // When a menu of the app's opened (it draws `data-oxplow-menu`), it is the
  // answer: the bubble goes. A press that opened none keeps its hint.
  const afterMenu = () => {
    setTimeout(() => {
      if (doc.querySelector("[data-oxplow-menu]")) hide();
    }, 0);
  };
  const onContextMenu = (e: Event) => {
    if (sending) return;
    if (start !== null) {
      // A finger's press: the browser's own long press.
      if (menuSent) {
        // Ours went first: this press has its menu.
        e.preventDefault();
        e.stopImmediatePropagation();
        return;
      }
      menuSent = true;
      // The browser's menu leaves the finger on the item it opened from; a
      // tap on the menu is a tap.
      endSwallow();
    }
    afterMenu();
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
