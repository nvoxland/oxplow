import { useEffect, useRef, useState, type ReactNode } from "react";

interface NarrowRailProps {
  /** True while the window is too narrow for the rail to sit beside the page. */
  narrow: boolean;
  /** Renders the rail; `close` hides the overlay (a no-op when not narrow). */
  children: (close: () => void) => ReactNode;
}

/**
 * Wide window: the rail in its normal place. Narrow window: a small button
 * at the left edge shows the rail as an overlay above the page (the page
 * keeps its width). Escape, a click outside, or opening a page hides it;
 * widening the window resets it.
 */
export function NarrowRail({ narrow, children }: NarrowRailProps) {
  const [open, setOpen] = useState(false);
  const overlayRef = useRef<HTMLDivElement>(null);
  const showing = narrow && open;

  useEffect(() => {
    if (!narrow) setOpen(false);
  }, [narrow]);

  useEffect(() => {
    if (!showing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      if (overlayRef.current && !overlayRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [showing]);

  if (!narrow) return <>{children(() => {})}</>;
  return (
    <>
      {showing ? null : (
        <button
          type="button"
          data-testid="rail-show"
          aria-label="Show the rail"
          title="Show the rail"
          onClick={() => setOpen(true)}
          style={{
            position: "absolute",
            left: 0,
            bottom: 8,
            zIndex: 20,
            width: 22,
            height: 28,
            padding: 0,
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            background: "var(--surface-chrome)",
            color: "var(--text-secondary)",
            border: "1px solid var(--border-strong)",
            borderLeft: "none",
            borderRadius: "0 6px 6px 0",
            cursor: "pointer",
          }}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden fill="none" stroke="currentColor" strokeWidth="1.5">
            <rect x="1" y="1.5" width="10" height="9" rx="1.5" />
            <line x1="4.5" y1="1.5" x2="4.5" y2="10.5" />
          </svg>
        </button>
      )}
      {showing ? (
        <div
          ref={overlayRef}
          data-testid="rail-overlay"
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            bottom: 0,
            zIndex: 30,
            boxShadow: "0 0 16px var(--surface-overlay)",
          }}
        >
          {children(() => setOpen(false))}
        </div>
      ) : null}
    </>
  );
}
