import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";

/**
 * The one dialog for anything that comes from within a tab: a page's
 * About, a page or viz action's form. It lives inside the tab's body area
 * (the tab body is its positioned ancestor, so it dims and covers only
 * that tab's content, never the tab strip, the rail HUD or the other
 * tabs), is as tall as its content, and is headed by a title (and, under
 * it, what it is about) and a × close.
 *
 * Because every tab body stays mounted while another tab is shown
 * (`CenterTabs`: `display: none`), a modal open in a tab is still open,
 * with a form's typed values, when the person comes back to the tab: its
 * state is the tab body's own React state, nothing more.
 *
 * Escape (anywhere in it), × and the content's own Cancel close it; a
 * click outside does not, so a half-filled form isn't lost. Focus moves
 * into it (the first field, else the dialog) and returns to what had it
 * when it closes.
 */
export function TabModal({
  title,
  subtitle,
  onClose,
  children,
  testId = "tab-modal",
  width = "min(480px, 100%)",
}: {
  title: ReactNode;
  /** What it is about, under the title, in words (never an id). */
  subtitle?: ReactNode;
  onClose(): void;
  children: ReactNode;
  testId?: string;
  /** CSS width of the card. */
  width?: string;
}) {
  const card = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const node = card.current;
    if (node && !node.contains(document.activeElement)) {
      // A field that took focus itself (autofocus) keeps it.
      const first = node.querySelector<HTMLElement>("input, textarea, select, [contenteditable='true']");
      (first ?? node).focus();
    }
    return () => {
      if (before && before.isConnected) before.focus();
    };
  }, []);
  return (
    <div data-testid={`${testId}-backdrop`} style={backdropStyle}>
      <div
        ref={card}
        role="dialog"
        aria-label={typeof title === "string" ? title : undefined}
        data-testid={testId}
        tabIndex={-1}
        style={{ ...cardStyle, width }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            e.preventDefault();
            onClose();
          }
        }}
      >
        <header style={headerStyle}>
          <div style={{ display: "flex", flexDirection: "column", minWidth: 0 }}>
            <span data-testid={`${testId}-title`} style={titleStyle}>
              {title}
            </span>
            {subtitle ? (
              <span data-testid={`${testId}-subtitle`} style={subtitleStyle}>
                {subtitle}
              </span>
            ) : null}
          </div>
          <button type="button" aria-label="Close" title="Close (Esc)" data-testid={`${testId}-close`} onClick={onClose} style={closeStyle}>
            ×
          </button>
        </header>
        <div style={bodyStyle}>{children}</div>
      </div>
    </div>
  );
}

const backdropStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  zIndex: 20,
  background: "rgba(0, 0, 0, 0.35)",
  display: "flex",
  justifyContent: "center",
  alignItems: "flex-start",
  padding: "48px 16px 16px",
  overflow: "hidden",
};

const cardStyle: CSSProperties = {
  // The dock panel's chrome (`Dock/DockPanel.tsx`): same surface, border and ×.
  background: "var(--bg-1)",
  color: "var(--text-primary, var(--fg))",
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  boxShadow: "0 8px 24px rgba(0, 0, 0, 0.28)",
  display: "flex",
  flexDirection: "column",
  maxHeight: "100%",
  outline: "none",
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "space-between",
  gap: 8,
  padding: "6px 12px 6px 16px",
  borderBottom: "1px solid var(--border-subtle)",
  flexShrink: 0,
};

const titleStyle: CSSProperties = {
  fontSize: "var(--text-base)",
  fontWeight: 600,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const subtitleStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary, var(--muted))",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const closeStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: "var(--text-secondary, var(--muted))",
  fontSize: 16,
  lineHeight: 1,
  cursor: "pointer",
  padding: "2px 4px",
};

const bodyStyle: CSSProperties = { minHeight: 0, overflow: "auto", padding: 16 };
