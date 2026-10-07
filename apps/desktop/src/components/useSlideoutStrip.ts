import type { MouseEvent as ReactMouseEvent } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

/**
 * Shared open/close state machine for a **slide-out glyph strip**: a thin
 * always-visible column of icons with a wider panel that slides over it (and
 * over whatever sits to its right) showing the same rows with full labels.
 * Used by the far-left `Navigator` (streams + threads) and the Terminal
 * page's `TerminalTabStrip`.
 *
 * Only the *behavior* lives here. Rows, groups, footers, widths, and
 * selection treatment stay in each component — those genuinely differ (one
 * strip is a two-level hierarchy with inline forms, the other a flat list),
 * and folding them together would need a component with a dozen props and
 * two render branches.
 *
 * Three rules this encodes, each of which was gotten wrong at least once
 * when the logic was hand-written per strip (tsk269, tsk270):
 *
 * 1. **Hover does not open.** The panel expands only on an explicit click.
 *    A strip that expands on `mouseEnter` fires whenever the pointer merely
 *    drifts across it en route somewhere else, and since the panel covers
 *    its neighbor, the accidental open buries the very thing the user was
 *    reaching for. A dwell delay only makes an involuntary action slower.
 *    See `.context/usability.md` → "Hover reveals, click rearranges".
 *
 * 2. **Pointer-leave is geometric, never `mouseleave`.** The panel is
 *    absolutely positioned over a sibling but is still a DOM descendant of
 *    the strip's own wrapper, so the pointer never "leaves" that wrapper
 *    while parked over the covered region — `mouseleave` simply never
 *    fires there, and the panel strands itself open on top of the thing
 *    it's covering, swallowing clicks meant for it. Comparing the pointer
 *    against the panel's rect has no such blind spot.
 *
 *    And the pointer has to have *been* in the panel before leaving it can
 *    close it. A panel opened from somewhere else — the title bar's
 *    stream name, above it — starts with the pointer outside; treating
 *    that as "left" flashed it open and shut on the first mouse move.
 *
 * 3. **Passive closes yield to an in-flight form; explicit ones don't.**
 *    Pointer-leave and background-click must not discard a rename or a
 *    half-typed new-item entry — pass `guard` while one is open. Escape,
 *    an outside press, and an explicit `closePanel()` are the user actively
 *    dismissing, so those always go through.
 */

/** Grace before a pointer that has left the panel actually closes it, so
 *  crossing a seam or overshooting by a few pixels doesn't snap it shut. */
const CLOSE_GRACE_MS = 180;

/** Anything resolving to one of these was meant to be operated, not
 *  clicked *past* — so it never counts as a background dismissal. */
const INTERACTIVE_SELECTOR = "button, input, select, textarea, a, label, [role='button']";

export function isInteractiveTarget(el: Element | null): boolean {
  return !!el?.closest(INTERACTIVE_SELECTOR);
}

/** Edges count as inside: the pointer resting exactly on the panel's
 *  boundary is still on the panel, and closing there fights the cursor. */
export function isPointInRect(
  rect: Pick<DOMRect, "left" | "top" | "right" | "bottom">,
  x: number,
  y: number,
): boolean {
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

export interface UseSlideoutStripOptions {
  /** While true, the two *passive* close paths (pointer-leave, background
   *  click) are suppressed because the user is mid-form inside the panel. */
  guard?: boolean;
  /** Fired whenever the panel transitions open → closed, by any route.
   *  Use it to drop transient panel-local state (an open rename, say). */
  onClose?(): void;
}

export interface SlideoutStrip {
  open: boolean;
  openPanel(): void;
  closePanel(): void;
  /** Attach to the panel element: `ref={strip.panelRef}`. Both the
   *  geometric leave test and outside-press containment read it. */
  panelRef(el: HTMLElement | null): void;
  /** Spread onto the panel element — background-click dismissal. */
  panelProps: { onClick(e: ReactMouseEvent<HTMLElement>): void };
  /** Spread onto the strip's dead-space container — clicking the strip's
   *  own background expands. A secondary affordance: a scrolling list has
   *  no dead space left, so the strip still needs a real toggle control. */
  deadSpaceProps: { onClick(e: ReactMouseEvent<HTMLElement>): void };
}

export function useSlideoutStrip(options: UseSlideoutStripOptions = {}): SlideoutStrip {
  const { guard = false, onClose } = options;
  const [open, setOpen] = useState(false);

  const panelElRef = useRef<HTMLElement | null>(null);
  const timerRef = useRef<number | null>(null);
  // Mirrors of live values so the callbacks below stay referentially
  // stable without going stale — the listener effects re-register on
  // `open`/`guard` only, not on every render of the host component.
  const openRef = useRef(false);
  /** The pointer has been inside the panel since it opened (rule 2). */
  const enteredRef = useRef(false);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const cancelClose = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const closePanel = useCallback(() => {
    cancelClose();
    if (!openRef.current) return;
    openRef.current = false;
    setOpen(false);
    onCloseRef.current?.();
  }, [cancelClose]);

  const openPanel = useCallback(() => {
    cancelClose();
    if (!openRef.current) enteredRef.current = false;
    openRef.current = true;
    setOpen(true);
  }, [cancelClose]);

  const scheduleClose = useCallback(() => {
    cancelClose();
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      closePanel();
    }, CLOSE_GRACE_MS);
  }, [cancelClose, closePanel]);

  useEffect(() => () => cancelClose(), [cancelClose]);

  // Explicit dismissal. Deliberately ignores `guard`: the user pressing
  // Escape or clicking away is saying "go away", which outranks "you might
  // be mid-form". Containment is tested against the PANEL rather than the
  // wrapper — the panel covers the strip while open, so "outside the panel"
  // and "outside the whole strip" are the same region, and the panel is the
  // element that would otherwise intercept the press.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closePanel();
    };
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (target && panelElRef.current?.contains(target)) return;
      closePanel();
    };
    document.addEventListener("keydown", onKey);
    // Capture phase so we collapse before the press reaches (and is
    // consumed by) whatever is beneath the panel.
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onPointerDown, true);
    };
  }, [open, closePanel]);

  // Passive dismissal — the pointer wandering off. See rule 2 above for why
  // this is a rect test rather than `mouseleave`.
  useEffect(() => {
    if (!open) return;
    const onMove = (e: PointerEvent) => {
      if (guard) {
        cancelClose();
        return;
      }
      const rect = panelElRef.current?.getBoundingClientRect();
      if (!rect) return;
      if (isPointInRect(rect, e.clientX, e.clientY)) {
        enteredRef.current = true;
        cancelClose();
      } else if (enteredRef.current) {
        scheduleClose();
      }
    };
    document.addEventListener("pointermove", onMove);
    return () => {
      document.removeEventListener("pointermove", onMove);
      cancelClose();
    };
  }, [open, guard, cancelClose, scheduleClose]);

  const panelRef = useCallback((el: HTMLElement | null) => {
    panelElRef.current = el;
  }, []);

  const onPanelClick = useCallback(
    (e: ReactMouseEvent<HTMLElement>) => {
      if (guard) return;
      if (isInteractiveTarget(e.target as Element | null)) return;
      closePanel();
    },
    [guard, closePanel],
  );

  const onDeadSpaceClick = useCallback(
    (e: ReactMouseEvent<HTMLElement>) => {
      // Only the container's own background — a click that bubbled up from
      // a row is that row's business, not an expand gesture.
      if (e.target === e.currentTarget) openPanel();
    },
    [openPanel],
  );

  return useMemo(
    () => ({
      open,
      openPanel,
      closePanel,
      panelRef,
      panelProps: { onClick: onPanelClick },
      deadSpaceProps: { onClick: onDeadSpaceClick },
    }),
    [open, openPanel, closePanel, panelRef, onPanelClick, onDeadSpaceClick],
  );
}
