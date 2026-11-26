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
/** How long after the lift what the browser sends is still the press's
 *  (its mouse events, a late menu of its own). */
const AFTER_LIFT_MS = 400;
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
  /** The finger's press: from its pointerdown to a little after its lift
   *  (`AFTER_LIFT_MS`), so what the browser sends at the lift is still its. */
  type Press = {
    target: Element;
    at: { x: number; y: number };
    /** It had its menu (the browser's or ours): no second one. */
    menuSent: boolean;
    /** A hint or a menu showed: the lift's mouse events are the hold's. */
    held: boolean;
    timers: ReturnType<typeof setTimeout>[];
  };
  let press: Press | null = null;
  let ending: ReturnType<typeof setTimeout> | null = null;
  let sending = false;
  let bubble: HTMLElement | null = null;

  const hide = () => {
    bubble?.remove();
    bubble = null;
  };
  /** The hold is over (a move, a scroll, a drag, the lift): no hint, no
   *  menu, but the press stays known until it ends. */
  const stopTimers = () => {
    for (const t of press?.timers ?? []) clearTimeout(t);
    if (press) press.timers = [];
  };
  const end = () => {
    stopTimers();
    if (ending !== null) clearTimeout(ending);
    ending = null;
    press = null;
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

  // When a menu of the app's opened (it draws `data-oxplow-menu`), it is the
  // answer: the bubble goes. A press that opened none keeps its hint.
  const afterMenu = () => {
    setTimeout(() => {
      if (doc.querySelector("[data-oxplow-menu]")) hide();
    }, 0);
  };

  const onDown = (e: PointerEvent) => {
    end();
    hide();
    if (e.pointerType !== "touch") return;
    const target = e.target instanceof Element ? e.target : null;
    if (!target) return;
    const p: Press = { target, at: { x: e.clientX, y: e.clientY }, menuSent: false, held: false, timers: [] };
    press = p;
    const text = hintFor(target);
    if (text !== null) {
      p.timers.push(
        setTimeout(() => {
          if (p.menuSent) return;
          show(target, text);
          p.held = true;
        }, HOLD_MS),
      );
    }
    // Where the OS's own long press is the right one (text), and inside an
    // open menu (a slow tap on an item is a tap), none of ours.
    if (!shouldSuppressContextMenu(describeChain(target)) || target.closest("[data-oxplow-menu]")) return;
    p.timers.push(
      setTimeout(() => {
        if (p.menuSent || !target.isConnected) return;
        p.menuSent = true;
        p.held = true;
        sending = true;
        try {
          target.dispatchEvent(
            new MouseEvent("contextmenu", { bubbles: true, cancelable: true, composed: true, button: 2, clientX: p.at.x, clientY: p.at.y }),
          );
        } finally {
          sending = false;
        }
        afterMenu();
      }, MENU_MS),
    );
  };
  const onMove = (e: PointerEvent) => {
    if (press && Math.hypot(e.clientX - press.at.x, e.clientY - press.at.y) > SLOP_PX) stopTimers();
  };
  // The lift: the hold is over; the press ends a little later, so the
  // mouse events and a menu the browser sends at the lift are still its.
  const onUp = () => {
    if (!press) return;
    stopTimers();
    if (ending !== null) clearTimeout(ending);
    ending = setTimeout(end, AFTER_LIFT_MS);
  };
  // A hold showed a hint or a menu: the press was a question, not a press.
  const onMouse = (e: MouseEvent) => {
    if (!press?.held) return;
    e.preventDefault();
    e.stopPropagation();
  };
  const onContextMenu = (e: Event) => {
    if (sending || !press) {
      if (!sending) afterMenu();
      return;
    }
    if (press.menuSent) {
      // This press had its menu already: no second one.
      e.preventDefault();
      e.stopImmediatePropagation();
      return;
    }
    // The browser's own long press: it is the press's menu, and no hint
    // draws over it.
    press.menuSent = true;
    press.held = true;
    stopTimers();
    hide();
    afterMenu();
  };
  const onDrag = () => {
    stopTimers();
    hide();
    if (press) press.held = false;
  };
  // Only a scroll of what holds the finger ends the hold (a log streaming
  // in another pane does not).
  const onScroll = (e: Event) => {
    if (!press) return;
    const node = e.target;
    const scrolled = node instanceof Document ? doc.documentElement : node instanceof Node ? node : null;
    if (scrolled && !scrolled.contains(press.target)) return;
    stopTimers();
    hide();
  };

  const listeners: Array<[string, (e: never) => void]> = [
    ["pointerdown", onDown],
    ["pointermove", onMove],
    ["pointerup", onUp],
    ["pointercancel", onUp],
    ["mousedown", onMouse],
    ["mouseup", onMouse],
    ["click", onMouse],
    ["contextmenu", onContextMenu],
    ["scroll", onScroll],
    ["dragstart", onDrag],
  ];
  for (const [type, fn] of listeners) doc.addEventListener(type, fn as EventListener, true);
  return () => {
    end();
    hide();
    for (const [type, fn] of listeners) doc.removeEventListener(type, fn as EventListener, true);
  };
}
