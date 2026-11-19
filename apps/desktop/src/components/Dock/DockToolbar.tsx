/// A dock panel's toolbar: the bar under its header in the page nav bar's
/// colors (`--panel-header-bg` over the card), and its buttons in the nav
/// bar's quiet style — no face or border at rest, the secondary text; a soft
/// tint and the primary text under the pointer. The Context and Alerts panels
/// draw their actions with these, so every dock panel's actions read alike.

import { useState, type ButtonHTMLAttributes, type CSSProperties, type ReactNode } from "react";

export const dockToolbarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  flexShrink: 0,
  minHeight: 32,
  padding: "4px 12px",
  borderBottom: "1px solid var(--border-strong, var(--border-subtle))",
  background: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)",
};

export function DockToolbar({ children, testId }: { children: ReactNode; testId?: string }) {
  return (
    <div data-testid={testId} style={dockToolbarStyle}>
      {children}
    </div>
  );
}

/** A toolbar button's look, under the pointer or not. */
export function barButtonStyle(hover: boolean): CSSProperties {
  return {
    display: "inline-flex",
    alignItems: "center",
    gap: 4,
    padding: "3px 8px",
    border: "none",
    borderRadius: 4,
    background: hover ? "rgba(255, 255, 255, 0.07)" : "transparent",
    color: hover ? "var(--text-primary)" : "var(--text-secondary)",
    font: "inherit",
    fontSize: "var(--text-xs)",
    cursor: "pointer",
  };
}

export function BarButton({ children, disabled, style, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { "data-testid"?: string }) {
  const [hover, setHover] = useState(false);
  return (
    <button type="button" {...rest} disabled={disabled} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)} style={{ ...barButtonStyle(hover && !disabled), ...style }}>
      {children}
    </button>
  );
}
