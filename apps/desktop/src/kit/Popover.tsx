import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";

export interface PopoverProps {
  /** What opened it: it sits beside it, and gets focus back on close. */
  anchor: RefObject<HTMLElement | null>;
  /** Its name for a screen reader. */
  label: string;
  onClose(): void;
  children: ReactNode;
  testId?: string;
}

/** The widest a popover gets, px. */
const MAX_WIDTH = 420;
/** Its gap from its anchor and from the window's edge, px. */
const GAP = 8;

/**
 * Room a control's row doesn't have: a dialog over the page, beside its
 * anchor and opening away from the rail (to the anchor's left when there
 * is space, else below it). Escape or a click outside closes it, and focus
 * goes back to the anchor. It stays in its owner's DOM, so focus moving
 * into it is still focus inside the owner.
 */
export function Popover({ anchor, label, onClose, children, testId }: PopoverProps) {
  const self = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<CSSProperties>({ visibility: "hidden" });
  const close = () => {
    onClose();
    anchor.current?.focus();
  };
  const closeRef = useRef(close);
  closeRef.current = close;

  useLayoutEffect(() => {
    const a = anchor.current?.getBoundingClientRect();
    if (!a) return;
    const width = Math.min(MAX_WIDTH, window.innerWidth - 2 * GAP);
    const left = a.left - GAP - width >= GAP ? a.left - GAP - width : Math.max(GAP, Math.min(a.left, window.innerWidth - width - GAP));
    const top = a.left - GAP - width >= GAP ? a.top : a.bottom + GAP;
    setAt({ left, top: Math.max(GAP, top), maxWidth: width });
  }, [anchor]);

  useEffect(() => {
    const outside = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (target && !self.current?.contains(target) && !anchor.current?.contains(target)) closeRef.current();
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [anchor]);

  return (
    <div
      ref={self}
      role="dialog"
      aria-label={label}
      className="ox-popover"
      data-testid={testId}
      style={at}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      {children}
    </div>
  );
}
