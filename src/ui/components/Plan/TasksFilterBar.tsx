import type { CSSProperties } from "react";
import type { WorkItemPriority, WorkItemStatus } from "../../api.js";

export interface TasksFilters {
  search: string;
  statuses: ReadonlySet<WorkItemStatus>;
  priorities: ReadonlySet<WorkItemPriority>;
  hideAuto: boolean;
  showClosed: boolean;
}

const PRIMARY_STATUSES: WorkItemStatus[] = ["ready", "in_progress", "blocked"];
const PRIORITIES: WorkItemPriority[] = ["urgent", "high", "medium", "low"];

const barStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexWrap: "wrap",
  padding: "6px 10px",
  borderBottom: "1px solid var(--border)",
  background: "var(--bg-2)",
  fontSize: 12,
};

const chipStyle: CSSProperties = {
  border: "1px solid var(--border)",
  borderRadius: 12,
  padding: "1px 8px",
  background: "var(--bg-1)",
  cursor: "pointer",
  userSelect: "none",
};

const chipOnStyle: CSSProperties = {
  ...chipStyle,
  background: "var(--accent-soft-bg, var(--accent))",
  color: "var(--accent-on, #fff)",
  borderColor: "var(--accent)",
};

const inputStyle: CSSProperties = {
  flex: "0 0 200px",
  padding: "2px 6px",
  border: "1px solid var(--border)",
  borderRadius: 4,
  background: "var(--bg-1)",
  color: "inherit",
  fontSize: 12,
};

/**
 * Filter bar shown above the Tasks page list. Holds search, status
 * chips, priority chips, an auto-filed toggle, and a "show closed"
 * toggle. State is owned by the parent (TasksPage) and persisted in
 * localStorage so it survives reloads.
 *
 * The chip sets default to empty (everything visible). When a chip is
 * toggled on, only items matching the active set show. When all chips
 * are off, the filter is inactive (no client-side filtering).
 */
export function TasksFilterBar({
  filters,
  onChange,
}: {
  filters: TasksFilters;
  onChange(next: TasksFilters): void;
}) {
  const toggleStatus = (s: WorkItemStatus) => {
    const next = new Set(filters.statuses);
    if (next.has(s)) next.delete(s); else next.add(s);
    onChange({ ...filters, statuses: next });
  };
  const togglePriority = (p: WorkItemPriority) => {
    const next = new Set(filters.priorities);
    if (next.has(p)) next.delete(p); else next.add(p);
    onChange({ ...filters, priorities: next });
  };
  return (
    <div style={barStyle} data-testid="tasks-filter-bar">
      <input
        type="search"
        placeholder="Search…"
        value={filters.search}
        onChange={(e) => onChange({ ...filters, search: e.target.value })}
        style={inputStyle}
        data-testid="tasks-filter-search"
      />
      <span style={{ color: "var(--muted)" }}>Status:</span>
      {PRIMARY_STATUSES.map((s) => (
        <button
          key={s}
          type="button"
          onClick={() => toggleStatus(s)}
          style={filters.statuses.has(s) ? chipOnStyle : chipStyle}
          data-testid={`tasks-filter-status-${s}`}
        >
          {s.replace("_", " ")}
        </button>
      ))}
      <span style={{ color: "var(--muted)" }}>·</span>
      <span style={{ color: "var(--muted)" }}>Priority:</span>
      {PRIORITIES.map((p) => (
        <button
          key={p}
          type="button"
          onClick={() => togglePriority(p)}
          style={filters.priorities.has(p) ? chipOnStyle : chipStyle}
          data-testid={`tasks-filter-priority-${p}`}
        >
          {p}
        </button>
      ))}
      <span style={{ flex: 1 }} />
      <label style={{ display: "flex", alignItems: "center", gap: 4, cursor: "pointer" }}>
        <input
          type="checkbox"
          checked={filters.hideAuto}
          onChange={(e) => onChange({ ...filters, hideAuto: e.target.checked })}
          data-testid="tasks-filter-hide-auto"
        />
        hide auto
      </label>
      <label style={{ display: "flex", alignItems: "center", gap: 4, cursor: "pointer" }}>
        <input
          type="checkbox"
          checked={filters.showClosed}
          onChange={(e) => onChange({ ...filters, showClosed: e.target.checked })}
          data-testid="tasks-filter-show-closed"
        />
        show closed
      </label>
    </div>
  );
}

export const DEFAULT_TASKS_FILTERS: TasksFilters = {
  search: "",
  statuses: new Set(),
  priorities: new Set(),
  hideAuto: false,
  showClosed: false,
};

const STORAGE_KEY = "tasks-filters";

export function loadTasksFilters(): TasksFilters {
  if (typeof window === "undefined") return DEFAULT_TASKS_FILTERS;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_TASKS_FILTERS;
    const parsed = JSON.parse(raw) as {
      search?: string;
      statuses?: string[];
      priorities?: string[];
      hideAuto?: boolean;
      showClosed?: boolean;
    };
    return {
      search: typeof parsed.search === "string" ? parsed.search : "",
      statuses: new Set((parsed.statuses ?? []) as WorkItemStatus[]),
      priorities: new Set((parsed.priorities ?? []) as WorkItemPriority[]),
      hideAuto: Boolean(parsed.hideAuto),
      showClosed: Boolean(parsed.showClosed),
    };
  } catch {
    return DEFAULT_TASKS_FILTERS;
  }
}

export function saveTasksFilters(filters: TasksFilters): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({
      search: filters.search,
      statuses: [...filters.statuses],
      priorities: [...filters.priorities],
      hideAuto: filters.hideAuto,
      showClosed: filters.showClosed,
    }));
  } catch { /* ignore quota */ }
}

export function applyTasksFilters<T extends { title: string; description: string; status: WorkItemStatus; priority: WorkItemPriority }>(
  items: T[],
  filters: TasksFilters,
): T[] {
  const q = filters.search.trim().toLowerCase();
  return items.filter((item) => {
    if (q && !(item.title.toLowerCase().includes(q) || item.description.toLowerCase().includes(q))) return false;
    if (filters.statuses.size > 0 && !filters.statuses.has(item.status)) return false;
    if (filters.priorities.size > 0 && !filters.priorities.has(item.priority)) return false;
    return true;
  });
}
