import type { CSSProperties } from "react";

/// The look of a touch bar under a pane on a touch screen (the terminal's,
/// the code editor's): a strip of keys a finger can press (44px, the touch
/// target size).

export const touchBarStyle: CSSProperties = {
  flex: "none",
  display: "flex",
  flexDirection: "column",
  gap: "var(--space-2)",
  padding: "var(--space-2)",
  background: "var(--surface-chrome)",
  borderTop: "1px solid var(--border-subtle)",
};

export const touchRowStyle: CSSProperties = { display: "flex", flexWrap: "wrap", gap: "var(--space-1)" };

export const touchKeyStyle = (on: boolean): CSSProperties => ({
  minWidth: 44,
  minHeight: 44,
  padding: "0 var(--space-3)",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--text-md)",
  color: "var(--text-primary)",
  background: on ? "var(--accent-soft-bg)" : "var(--surface-elevated)",
  border: `1px solid ${on ? "var(--accent)" : "var(--border-strong)"}`,
  borderRadius: "var(--radius-md)",
  touchAction: "manipulation",
});
