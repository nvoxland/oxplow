import type { CSSProperties } from "react";
import type { ThreadWorkState, Task } from "../../api.js";

const paneStyle: CSSProperties = {
  flex: "0 0 280px",
  borderLeft: "1px solid var(--border)",
  background: "var(--bg-1)",
  padding: "10px 12px",
  display: "flex",
  flexDirection: "column",
  gap: 12,
  fontSize: "var(--text-xs)",
  overflowY: "auto",
};

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "baseline",
  justifyContent: "space-between",
  gap: 8,
};

const numberStyle: CSSProperties = {
  fontSize: 18,
  fontWeight: 600,
  color: "var(--accent)",
};

/**
 * Right-side detail companion to TasksList. When no row is selected,
 * shows a compact summary of the active thread's work: counts, oldest
 * blocked item age, and the most recent transitions / completions.
 *
 * Intentionally does not render an editable row detail today —
 * editing happens through the existing modal path (PlanPane's
 * TaskDetail). The summary view fills the space so the page
 * feels informative when nothing is selected, which is the most
 * common state on a quiet thread.
 */
/// Derive the summary view from the backend's bucketed work state.
/// `items` carries only Ready tasks — in_progress / blocked / done
/// rows arrive solely via their own buckets, so counting by filtering
/// `items` would pin those numbers at zero. The done bucket also
/// folds in canceled/archived rows; the summary counts real
/// completions only. Exported for tests.
export function summarizeThreadWork(threadWork: ThreadWorkState | null): {
  counts: { inProgress: number; ready: number; blocked: number; done: number };
  oldestBlocked: Task | null;
  recentDone: Task[];
} {
  const inProgress = threadWork?.inProgress ?? [];
  const ready = threadWork?.items ?? [];
  const blocked = threadWork?.waiting ?? [];
  const done = (threadWork?.done ?? []).filter((i) => i.status === "done");
  return {
    counts: {
      inProgress: inProgress.length,
      ready: ready.length,
      blocked: blocked.length,
      done: done.length,
    },
    oldestBlocked: pickOldestBlocked(blocked),
    recentDone: pickRecentlyClosed(done, 3),
  };
}

export function TaskDetailPane({
  threadWork,
}: {
  threadWork: ThreadWorkState | null;
}) {
  const { counts, oldestBlocked, recentDone: recent } = summarizeThreadWork(threadWork);
  return (
    <div style={paneStyle} data-testid="task-detail-pane">
      <div style={{ fontWeight: 600, color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10 }}>
        Summary
      </div>
      <div style={rowStyle}>
        <span>In progress</span>
        <span style={numberStyle} data-testid="tasks-summary-in-progress">{counts.inProgress}</span>
      </div>
      <div style={rowStyle}>
        <span>To do</span>
        <span style={numberStyle} data-testid="tasks-summary-ready">{counts.ready}</span>
      </div>
      <div style={rowStyle}>
        <span>Blocked</span>
        <span style={numberStyle} data-testid="tasks-summary-blocked">{counts.blocked}</span>
      </div>
      <div style={rowStyle}>
        <span>Done</span>
        <span style={numberStyle} data-testid="tasks-summary-done">{counts.done}</span>
      </div>
      {oldestBlocked ? (
        <div style={{ borderTop: "1px solid var(--border)", paddingTop: 8 }}>
          <div style={{ color: "var(--muted)", fontSize: 11 }}>Oldest blocked</div>
          <div style={{ fontWeight: 600, marginTop: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {oldestBlocked.title}
          </div>
          <div style={{ color: "var(--muted)", fontSize: 11 }}>
            {formatAge(oldestBlocked.updated_at)} old
          </div>
        </div>
      ) : null}
      {recent.length > 0 ? (
        <div style={{ borderTop: "1px solid var(--border)", paddingTop: 8 }}>
          <div style={{ color: "var(--muted)", fontSize: 11, marginBottom: 4 }}>Recently completed</div>
          {recent.map((item) => (
            <div key={item.id} style={{ marginBottom: 4 }}>
              <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                ✓ {item.title}
              </div>
              <div style={{ color: "var(--muted)", fontSize: 11 }}>
                {formatAge(item.completed_at ?? item.updated_at)} ago
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function pickOldestBlocked(blocked: Task[]): Task | null {
  if (blocked.length === 0) return null;
  return blocked.reduce((best, cur) => (cur.updated_at < best.updated_at ? cur : best));
}

function pickRecentlyClosed(done: Task[], limit: number): Task[] {
  return [...done]
    .sort((a, b) => (b.completed_at ?? b.updated_at).localeCompare(a.completed_at ?? a.updated_at))
    .slice(0, limit);
}

function formatAge(iso: string | null): string {
  if (!iso) return "—";
  const ms = Date.now() - Date.parse(iso);
  if (Number.isNaN(ms) || ms < 0) return iso;
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}
