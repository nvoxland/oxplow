import type { WorkList } from "../../workItems.js";
import { openBacklogCount } from "./plan-utils.js";

const headerStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 10px",
  background: "var(--bg-2)",
  borderTop: "1px solid var(--border)",
  fontSize: "var(--text-xs)",
  userSelect: "none",
};

const openBtnStyle: React.CSSProperties = {
  fontSize: 11,
  padding: "1px 6px",
  border: "1px solid var(--border)",
  borderRadius: 4,
  background: "transparent",
  cursor: "pointer",
};

/**
 * Static footer at the bottom of the Tasks page surfacing the
 * stream-global backlog size, with a link to the full Backlog page
 * where grooming + promotion happens. No expandable body — the count
 * plus the "open ↗" link is the whole control.
 */
export function BacklogDrawer({
  backlog,
  onOpenBacklog,
}: {
  backlog: WorkList | null;
  onOpenBacklog(): void;
}) {
  const count = openBacklogCount(backlog);
  return (
    <div data-testid="tasks-backlog-drawer">
      <div style={headerStyle}>
        <span style={{ fontWeight: 600 }}>Backlog</span>
        <span style={{ color: "var(--muted)" }}>({count})</span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={onOpenBacklog}
          style={openBtnStyle}
          data-testid="tasks-backlog-drawer-open-page"
        >
          open ↗
        </button>
      </div>
    </div>
  );
}
