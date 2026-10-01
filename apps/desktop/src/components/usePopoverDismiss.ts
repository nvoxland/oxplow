import { useEffect, useRef } from "react";

/** A popover that closes on a pointer-down outside its box or on Escape,
 *  while `open`. Put the returned ref on the box holding the trigger and
 *  the popover. */
export function usePopoverDismiss<T extends HTMLElement>(open: boolean, close: () => void) {
  const boxRef = useRef<T | null>(null);
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) closeRef.current();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeRef.current();
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return boxRef;
}
