import type { CSSProperties } from "react";
import type { FieldDecl } from "../../workItems.js";
import type { FieldFilter } from "./plan-utils.js";

const barStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexWrap: "wrap",
  padding: "6px 10px",
  borderBottom: "1px solid var(--border)",
  background: "var(--bg-2)",
  fontSize: "var(--text-xs)",
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

/**
 * Filter bar above the Tasks page list: a row of chips for each enum
 * field the active list declares (priority, who filed it, …). Toggling a
 * chip on keeps the items with that value; a field with no chip on
 * doesn't filter. Renders nothing for a list with no enum fields.
 */
export function TasksFilterBar({
  fields,
  filter,
  onChange,
}: {
  fields: FieldDecl[];
  filter: FieldFilter;
  onChange(next: FieldFilter): void;
}) {
  const enums = fields.filter((f) => f.kind === "enum");
  if (enums.length === 0) return null;
  const toggle = (name: string, value: string) => {
    const current = filter[name] ?? [];
    const next = current.includes(value) ? current.filter((v) => v !== value) : [...current, value];
    onChange({ ...filter, [name]: next });
  };
  return (
    <div style={barStyle} data-testid="tasks-filter-bar">
      {enums.map((field) => (
        <span key={field.name} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
          <span style={{ color: "var(--muted)" }}>{field.title}:</span>
          {field.values.map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => toggle(field.name, value)}
              style={(filter[field.name] ?? []).includes(value) ? chipOnStyle : chipStyle}
              data-testid={`tasks-filter-${field.name}-${value}`}
            >
              {value.replace(/_/g, " ")}
            </button>
          ))}
        </span>
      ))}
    </div>
  );
}

const STORAGE_KEY = "tasks-filters";

/** The person's chips, kept per browser across reloads. */
export function loadTasksFilters(): FieldFilter {
  if (typeof window === "undefined") return {};
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, string[]> = {};
    for (const [name, values] of Object.entries(parsed as Record<string, unknown>)) {
      if (Array.isArray(values)) out[name] = values.filter((v): v is string => typeof v === "string");
    }
    return out;
  } catch {
    return {};
  }
}

export function saveTasksFilters(filter: FieldFilter): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(filter));
  } catch { /* ignore quota */ }
}
