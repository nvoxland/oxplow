import type { ReactNode } from "react";
import type { CanonicalState, FieldDecl } from "../workItems.js";
import { PriorityIcon, priorityColorVar } from "./Plan/plan-icons.js";

/**
 * A work list's own fields, rendered and edited from what it declares
 * (`v_capability_provider.fields`), whichever list is active: an `enum` is
 * a pill picker, `text` and `number` an input, a read-only field plain
 * text. An enum named `priority` draws with the priority glyph.
 */

/** A field's value as text ("" when unset). */
export function fieldText(value: unknown): string {
  return value === undefined || value === null ? "" : String(value);
}

/** A field's value as a row shows it: the priority glyph, else its text. */
export function FieldBadge({ field, value }: { field: FieldDecl; value: unknown }) {
  const text = fieldText(value);
  if (!text) return null;
  if (field.kind === "enum" && field.name === "priority") return <PriorityIcon priority={text} />;
  return (
    <span title={field.title} style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>
      {text.replace(/_/g, " ")}
    </span>
  );
}

/** A state's colour. */
export function stateColor(state: CanonicalState): string {
  switch (state) {
    case "in_progress": return "var(--status-running)";
    case "todo": return "var(--status-ready)";
    case "done": return "var(--status-done)";
    case "blocked": return "var(--status-waiting)";
    case "canceled": return "var(--status-canceled)";
  }
}

/** An enum value's colour: the priority scale for `priority`, else neutral. */
function enumColor(field: FieldDecl, value: string): string {
  return field.name === "priority" ? priorityColorVar(value) : "var(--border-strong, var(--border))";
}

/** One field, editable unless it's read-only. */
export function FieldPill({
  field,
  value,
  onChange,
}: {
  field: FieldDecl;
  value: unknown;
  onChange(value: string | number): void;
}) {
  const text = fieldText(value);
  if (field.read_only) return <span style={{ color: "var(--text-secondary)" }}>{text.replace(/_/g, " ") || "—"}</span>;
  if (field.kind === "enum") {
    return (
      <PillSelect
        value={text}
        options={field.values}
        color={enumColor(field, text)}
        label={field.title}
        onChange={onChange}
      />
    );
  }
  return (
    <input
      aria-label={field.title}
      defaultValue={text}
      type={field.kind === "number" ? "number" : "text"}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          (e.target as HTMLInputElement).value = text;
          (e.target as HTMLInputElement).blur();
        }
      }}
      onBlur={(e) => {
        const raw = e.target.value;
        if (raw === text) return;
        onChange(field.kind === "number" && raw !== "" ? Number(raw) : raw);
      }}
      style={{
        font: "inherit",
        fontSize: "var(--text-xs)",
        padding: "3px 8px",
        borderRadius: 6,
        border: "1px solid var(--border-subtle)",
        background: "var(--surface-card)",
        color: "var(--text-primary)",
        width: "100%",
      }}
    />
  );
}

/**
 * Colored pill that opens a native `<select>` on click. The native select
 * stays transparent over the pill so keyboard navigation + accessibility
 * come for free; the pill chrome is purely visual.
 */
export function PillSelect({
  value,
  options,
  color,
  label,
  render = (v) => v.replace(/_/g, " "),
  onChange,
}: {
  value: string;
  options: readonly string[];
  color: string;
  label?: string;
  render?: (value: string) => ReactNode;
  onChange(value: string): void;
}) {
  return (
    <span
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        padding: "3px 10px",
        borderRadius: 999,
        background: "var(--surface-card)",
        border: `1px solid ${color}`,
        color: "var(--text-primary)",
        fontSize: "var(--text-xs)",
        cursor: "pointer",
        minWidth: 0,
      }}
    >
      <span style={{ width: 8, height: 8, borderRadius: "50%", background: color, flexShrink: 0 }} aria-hidden />
      <span>{value ? render(value) : "—"}</span>
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        style={{ position: "absolute", inset: 0, opacity: 0, cursor: "pointer", width: "100%", height: "100%", font: "inherit" }}
      >
        {value ? null : <option value="">—</option>}
        {options.map((option) => (
          <option key={option} value={option}>
            {typeof render(option) === "string" ? (render(option) as string) : option.replace(/_/g, " ")}
          </option>
        ))}
      </select>
    </span>
  );
}
