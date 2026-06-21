/**
 * Generic daily bar chart for analytics pages. Each row is one bucket
 * (`label` is the x-axis key, e.g. a `YYYY-MM-DD` day; `value` the bar
 * height). Extracted from the old Go To `DailyChart` so both Page
 * Analytics (visits/day) and Token Analytics (tokens/day) share it.
 */
export interface DailyBarRow {
  label: string;
  value: number;
}

export function DailyBarChart({
  rows,
  emptyHint = "No data in range.",
  formatValue = (v) => String(v),
  summary,
}: {
  rows: DailyBarRow[];
  emptyHint?: string;
  /** Formats the total / peak figures + per-bar tooltip value. */
  formatValue?(value: number): string;
  /** Optional caption override; defaults to "<total> total · peak <peak>". */
  summary?: string;
}) {
  if (rows.length === 0) {
    return (
      <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)", fontStyle: "italic" }}>
        {emptyHint}
      </div>
    );
  }
  const max = Math.max(1, ...rows.map((r) => r.value));
  const total = rows.reduce((sum, r) => sum + r.value, 0);
  return (
    <div
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: "12px 14px",
      }}
    >
      <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", marginBottom: 8 }}>
        {summary ?? `${formatValue(total)} total · peak ${formatValue(max)}`}
      </div>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 2, height: 80 }}>
        {rows.map((r) => (
          <div
            key={r.label}
            title={`${r.label}: ${formatValue(r.value)}`}
            style={{
              flex: 1,
              minWidth: 4,
              height: `${Math.max(2, (r.value / max) * 100)}%`,
              background: "var(--accent-fg, #58a6ff)",
              borderRadius: "2px 2px 0 0",
              opacity: 0.85,
            }}
          />
        ))}
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", marginTop: 4, fontSize: 10, color: "var(--text-muted)" }}>
        <span>{rows[0]?.label}</span>
        <span>{rows[rows.length - 1]?.label}</span>
      </div>
    </div>
  );
}
