import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef } from "react";

/**
 * Backdrop-click rule, extracted so it's testable without a DOM. A
 * click on the backdrop closes the slideover; a click that bubbled up
 * from the panel does not. React's synthetic event reaches the handler
 * with `currentTarget` always pointing at the backdrop (the element the
 * handler is attached to), so we just check whether the click's actual
 * `target` is the same node.
 */
export function backdropShouldClose<T>({ target, currentTarget }: { target: T; currentTarget: T }): boolean {
  return target === currentTarget;
}

/**
 * Right-edge slideover panel — replaces centered modals for form-shaped
 * flows that warrant a focused workspace but don't justify a full page.
 *
 * Usability rules:
 *  - Anchored to the right edge of the host viewport (~30–40% width).
 *  - Backdrop click closes.
 *  - Escape closes.
 *  - Focus is moved into the panel on open (first focusable, falling back
 *    to the panel root) so keyboard users can tab through the form
 *    without re-orienting.
 *  - Body content scrolls; header + footer stay pinned.
 *
 * Layout note: Slideover is portal-free — it renders in-place and uses
 * `position: fixed` so it covers the entire viewport. Each host opens its
 * own Slideover; per-thread tab state does NOT track them (slideovers
 * are transient, like the legacy modals they replace).
 */
export function Slideover({
  open,
  onClose,
  title,
  width = "min(38vw, 560px)",
  testId,
  actions,
  footer,
  children,
}: {
  open: boolean;
  onClose(): void;
  title: ReactNode;
  /** CSS width value. Defaults to ~38vw capped at 560px. */
  width?: string;
  testId?: string;
  /** Header right-side actions (close button is rendered automatically). */
  actions?: ReactNode;
  /** Footer pinned to the bottom (typically primary action buttons). */
  footer?: ReactNode;
  children: ReactNode;
}) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!open) return;
    const node = panelRef.current;
    if (!node) return;
    // Move focus inside the panel so Escape/Tab feel natural.
    const first = node.querySelector<HTMLElement>(
      "input, textarea, select, button, [tabindex]:not([tabindex='-1'])",
    );
    if (first) {
      first.focus();
    } else {
      node.focus();
    }
  }, [open]);

  if (!open) return null;

  return (
    <div
      data-testid={testId ? `${testId}-backdrop` : "slideover-backdrop"}
      onClick={(event) => {
        if (backdropShouldClose({ target: event.target, currentTarget: event.currentTarget })) {
          onClose();
        }
      }}
      style={backdropStyle}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : undefined}
        data-testid={testId ?? "slideover"}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        style={{ ...panelStyle, width }}
      >
        <header style={headerStyle}>
          <span
            data-testid={testId ? `${testId}-title` : "slideover-title"}
            style={titleStyle}
          >
            {title}
          </span>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            {actions}
            <button
              type="button"
              aria-label="Close"
              data-testid={testId ? `${testId}-close` : "slideover-close"}
              onClick={onClose}
              style={closeBtnStyle}
            >
              ×
            </button>
          </div>
        </header>
        <div style={bodyStyle}>{children}</div>
        {footer ? <footer style={footerStyle}>{footer}</footer> : null}
      </div>
    </div>
  );
}

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.35)",
  display: "flex",
  justifyContent: "flex-end",
  alignItems: "stretch",
  zIndex: 1000,
};

const panelStyle: CSSProperties = {
  background: "var(--surface-card, var(--bg))",
  color: "var(--text-primary, var(--fg))",
  borderLeft: "1px solid var(--border-subtle, var(--border))",
  display: "flex",
  flexDirection: "column",
  boxShadow: "-8px 0 24px rgba(0, 0, 0, 0.18)",
  outline: "none",
  height: "100%",
  minWidth: 320,
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "12px 16px",
  borderBottom: "1px solid var(--border-subtle, var(--border))",
  flexShrink: 0,
  minHeight: 48,
};

const titleStyle: CSSProperties = {
  fontSize: 14,
  fontWeight: 600,
  color: "var(--text-primary, var(--fg))",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const closeBtnStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: "var(--text-secondary, var(--muted))",
  fontSize: 18,
  lineHeight: 1,
  cursor: "pointer",
  padding: "2px 8px",
};

const bodyStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflow: "auto",
  padding: 16,
};

const footerStyle: CSSProperties = {
  borderTop: "1px solid var(--border-subtle, var(--border))",
  padding: "10px 16px",
  display: "flex",
  alignItems: "center",
  justifyContent: "flex-end",
  gap: 8,
  flexShrink: 0,
};
