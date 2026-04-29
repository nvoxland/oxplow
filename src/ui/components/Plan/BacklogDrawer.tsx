import { useState } from "react";
import type { BacklogState, WorkItem } from "../../api.js";
import { runWithError } from "../../ui-error.js";

const headerStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 10px",
  background: "var(--bg-2)",
  borderTop: "1px solid var(--border)",
  fontSize: 12,
  cursor: "pointer",
  userSelect: "none",
};

const drawerBodyStyle: React.CSSProperties = {
  padding: "6px 10px 8px 10px",
  background: "var(--bg-1)",
  borderTop: "1px solid var(--border)",
  display: "flex",
  flexDirection: "column",
  gap: 4,
  maxHeight: 220,
  overflowY: "auto",
  fontSize: 12,
};

const rowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "3px 4px",
  borderRadius: 4,
};

const promoteBtnStyle: React.CSSProperties = {
  fontSize: 11,
  padding: "1px 6px",
  border: "1px solid var(--border)",
  borderRadius: 4,
  background: "var(--bg-2)",
  cursor: "pointer",
};

/**
 * Collapsible drawer at the bottom of the Tasks page surfacing the
 * stream-global backlog. The fast path for promoting a candidate into
 * the current thread (click the Promote button); the full grooming
 * surface stays on the Backlog page (header link).
 *
 * Default collapsed; open state persists in localStorage.
 */
export function BacklogDrawer({
  backlog,
  activeThreadId,
  onPromote,
  onOpenBacklog,
}: {
  backlog: BacklogState | null;
  activeThreadId: string | null;
  onPromote(itemId: string, toThreadId: string): Promise<void>;
  onOpenBacklog(): void;
}) {
  const [open, setOpen] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.localStorage.getItem("tasks-backlog-drawer-open") === "1";
  });
  const candidates: WorkItem[] = backlog?.waiting ?? [];
  const count = candidates.length;
  const toggle = () => {
    setOpen((prev) => {
      const next = !prev;
      try { window.localStorage.setItem("tasks-backlog-drawer-open", next ? "1" : "0"); } catch { /* ignore */ }
      return next;
    });
  };
  return (
    <div data-testid="tasks-backlog-drawer">
      <div
        style={headerStyle}
        onClick={toggle}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggle(); } }}
      >
        <span style={{ fontWeight: 600 }}>{open ? "▾" : "▸"} Backlog</span>
        <span style={{ color: "var(--muted)" }}>({count})</span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={(e) => { e.stopPropagation(); onOpenBacklog(); }}
          style={{ ...promoteBtnStyle, background: "transparent" }}
          data-testid="tasks-backlog-drawer-open-page"
        >
          open ↗
        </button>
      </div>
      {open ? (
        <div style={drawerBodyStyle} data-testid="tasks-backlog-drawer-body">
          {count === 0 ? (
            <div style={{ color: "var(--muted)", fontStyle: "italic" }}>
              Backlog is empty.
            </div>
          ) : (
            candidates.map((item) => (
              <div key={item.id} style={rowStyle}>
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {item.title}
                </span>
                {item.category ? (
                  <span style={{ color: "var(--muted)", fontSize: 11 }}>{item.category}</span>
                ) : null}
                <button
                  type="button"
                  disabled={!activeThreadId}
                  onClick={() => {
                    if (!activeThreadId) return;
                    void runWithError(
                      "Promote backlog item",
                      onPromote(item.id, activeThreadId),
                    );
                  }}
                  style={promoteBtnStyle}
                  title={activeThreadId ? "Promote to current thread" : "Select a thread first"}
                >
                  Promote →
                </button>
              </div>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}
