import type { ReactNode } from "react";

/**
 * Generic card shell — bordered, padded section with a header row that
 * carries a title on the left and an optional `action` slot on the
 * right. Used by dashboard-style pages (Git Dashboard, Plan Work) so
 * panels share one visual vocabulary.
 *
 * Extracted from GitDashboardPage's local helper when the Plan Work
 * page started reusing it; keep it dumb (no state, no styling
 * variants).
 */
export function Card({
  title,
  children,
  testId,
  action,
}: {
  title: string;
  children: ReactNode;
  testId?: string;
  action?: ReactNode;
}) {
  return (
    <section
      data-testid={testId}
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: 12,
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        <div style={{ fontWeight: 600, fontSize: "var(--text-base)" }}>{title}</div>
        {action}
      </div>
      {children}
    </section>
  );
}
