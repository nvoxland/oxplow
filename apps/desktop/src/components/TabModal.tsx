import { createContext, useContext, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** Where a tab's dialogs dock: the page's area under its nav bar (`Page`
 *  provides it), so a dialog sits at its top right and never scrolls with
 *  the body. With none (a dialog outside a page), it draws in place. */
export const TabModalHostContext = createContext<HTMLElement | null>(null);

/**
 * The one dialog for anything that comes from within a tab: a page's
 * About, a page or viz action's form. It docks at the right of the page's
 * area under its nav bar (`TabModalHostContext`), as tall as its content:
 * it dims only that page's body, never its nav bar, the tab strip, the
 * rail HUD or the other tabs, and is headed by a title (and, under it,
 * what it is about) and a × close.
 *
 * Because every tab body stays mounted while another tab is shown
 * (`CenterTabs`: `display: none`), a modal open in a tab is still open,
 * with a form's typed values, when the person comes back to the tab: its
 * state is the tab body's own React state, nothing more.
 *
 * Escape (anywhere in it), × and the content's own Cancel close it, and so
 * does a click outside it — until something in it was typed or changed:
 * then a click outside keeps it open, flashes its edge and says why, so a
 * half-filled form isn't lost to a stray click. Focus moves
 * into it (the first field, else the dialog) and returns to what had it
 * when it closes.
 */
export function TabModal({
  title,
  subtitle,
  onClose,
  children,
  testId = "tab-modal",
  width = "min(38vw, 560px)",
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
  const host = useContext(TabModalHostContext);
  // Something in it was typed or changed: a click outside no longer closes it.
  const [changed, setChanged] = useState(false);
  // A click outside it was refused (bumped to replay the flash).
  const [kept, setKept] = useState(0);
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
  const dialog = (
    <div
      data-testid={`${testId}-backdrop`}
      style={backdropStyle}
      onMouseDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (changed) setKept((n) => n + 1);
        else onClose();
      }}
    >
      <div
        ref={card}
        role="dialog"
        aria-label={typeof title === "string" ? title : undefined}
        data-testid={testId}
        tabIndex={-1}
        data-kept={kept > 0 ? "true" : undefined}
        style={{ ...cardStyle, width, ...(kept > 0 ? { animation: "oxplow-kept 0.6s ease-in-out" } : null) }}
        onInputCapture={() => setChanged(true)}
        onChangeCapture={() => setChanged(true)}
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
        {kept > 0 ? (
          <div key={kept} data-testid={`${testId}-kept`} role="status" style={keptStyle}>
            You changed something here, so it stays open. Close it with × or Cancel.
          </div>
        ) : null}
        <div style={bodyStyle}>{children}</div>
      </div>
    </div>
  );
  return host ? createPortal(dialog, host) : dialog;
}

const backdropStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  zIndex: 20,
  background: "rgba(0, 0, 0, 0.35)",
  display: "flex",
  justifyContent: "flex-end",
  alignItems: "flex-start",
  overflow: "hidden",
  pointerEvents: "auto",
};

const cardStyle: CSSProperties = {
  // The dock panel's chrome (`Dock/DockPanel.tsx`): same surface, border and ×.
  // The nav bar's background, so it reads as part of the chrome, apart from the page.
  background: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)",
  color: "var(--text-primary, var(--fg))",
  // Docked under the nav bar at the right, as tall as its content.
  borderLeft: "1px solid var(--border-strong)",
  borderBottom: "1px solid var(--border-strong)",
  borderBottomLeftRadius: 6,
  boxShadow: "-8px 8px 24px rgba(0, 0, 0, 0.28)",
  display: "flex",
  flexDirection: "column",
  maxHeight: "100%",
  outline: "none",
};

const keptStyle: CSSProperties = {
  padding: "6px 16px",
  fontSize: "var(--text-xs)",
  color: "var(--text-primary, var(--fg))",
  background: "color-mix(in srgb, var(--accent) 22%, transparent)",
  borderBottom: "1px solid var(--accent)",
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
