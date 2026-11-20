/// An empty page or section (P6.D2, `.context/usability.md` → "Empty
/// states"): what would be here and one sentence on how it gets
/// here. `compact` is the one-line form for rail sections.
import type { CSSProperties, ReactNode } from "react";

export function EmptyState({
  title,
  text,
  children,
  compact = false,
  testId,
}: {
  title: string;
  text?: ReactNode;
  /** Anything else the empty state offers (a "+ New" button). */
  children?: ReactNode;
  compact?: boolean;
  testId?: string;
}) {
  return (
    <div data-testid={testId} data-empty-state="" style={compact ? compactStyle : boxStyle}>
      <div style={compact ? undefined : { fontWeight: 600 }}>{title}</div>
      {text ? <div style={{ color: "var(--text-secondary)" }}>{text}</div> : null}
      {children}
    </div>
  );
}

const boxStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: "12px 0",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
};
const compactStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
};
