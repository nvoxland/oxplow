import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";

/** One input of an {@link InlinePromptStrip}. */
export interface PromptField {
  key: string;
  initialValue?: string;
  placeholder?: string;
  /** A textarea: Enter is a newline, Cmd/Ctrl+Enter submits. */
  multiline?: boolean;
  /** May be left empty (submits ""): clearing it is a choice. */
  optional?: boolean;
  /** Masked (a token, a key). */
  secret?: boolean;
}

/** A checkbox beside the fields; its submitted value is "true" or "". */
export interface PromptToggle {
  key: string;
  label: string;
  initial?: boolean;
}

/**
 * Inline prompt strip — the new-X flow's inline form (no modal): a
 * message, one or more fields, Cancel and a confirm button. Enter (or
 * Cmd/Ctrl+Enter in a multiline field) submits once every field has a
 * value; Escape in any field cancels. The first field is focused and
 * selected. The owner dismisses it, so it keeps what was typed while a
 * submit runs and after one fails; `error` is why one failed, shown under
 * the fields. Field `key`s name the submitted values (trimmed) and the
 * test ids (`<testId>-<key>`).
 */
export function InlinePromptStrip({
  message,
  fields,
  toggle,
  confirmLabel,
  onSubmit,
  onCancel,
  busy = false,
  error = null,
  testId,
  style,
}: {
  message: string;
  fields: PromptField[];
  toggle?: PromptToggle;
  confirmLabel: string;
  onSubmit(values: Record<string, string>): void;
  onCancel(): void;
  busy?: boolean;
  error?: string | null;
  testId?: string;
  style?: CSSProperties;
}) {
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(fields.map((f) => [f.key, f.initialValue ?? ""])),
  );
  const firstRef = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
  useEffect(() => {
    firstRef.current?.select();
  }, []);
  const [toggled, setToggled] = useState(!!toggle?.initial);
  const trimmed = Object.fromEntries(fields.map((f) => [f.key, (values[f.key] ?? "").trim()]));
  const ready = !busy && fields.every((f) => f.optional || trimmed[f.key]!.length > 0);
  const submit = () => {
    if (ready) onSubmit(toggle ? { ...trimmed, [toggle.key]: toggled ? "true" : "" } : trimmed);
  };
  const onKeyDown = (field: PromptField) => (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else if (field.multiline && e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    }
  };
  return (
    <form
      style={{ ...stripStyle, ...style }}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <div style={{ color: "var(--muted)", fontSize: 11 }}>{message}</div>
      <div style={{ display: "flex", gap: 6, alignItems: "flex-start", flexWrap: "wrap" }}>
        {fields.map((f, i) => {
          const common = {
            "data-testid": testId ? `${testId}-${f.key}` : undefined,
            "aria-label": f.placeholder ?? f.key,
            value: values[f.key] ?? "",
            placeholder: f.placeholder,
            autoFocus: i === 0,
            onKeyDown: onKeyDown(f),
            style: inputStyle,
          };
          return f.multiline ? (
            <textarea
              key={f.key}
              {...common}
              ref={i === 0 ? (n) => void (firstRef.current = n) : undefined}
              rows={3}
              onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
            />
          ) : (
            <input
              key={f.key}
              {...common}
              type={f.secret ? "password" : "text"}
              ref={i === 0 ? (n) => void (firstRef.current = n) : undefined}
              onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
            />
          );
        })}
        {toggle ? (
          <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, color: "var(--text-primary)" }}>
            <input
              type="checkbox"
              data-testid={testId ? `${testId}-${toggle.key}` : undefined}
              checked={toggled}
              onChange={(e) => setToggled(e.target.checked)}
            />
            {toggle.label}
          </label>
        ) : null}
        <button type="button" onClick={onCancel} style={buttonStyle}>
          Cancel
        </button>
        <button
          type="submit"
          data-testid={testId ? `${testId}-submit` : undefined}
          disabled={!ready}
          style={{ ...primaryStyle, opacity: ready ? 1 : 0.5 }}
        >
          {confirmLabel}
        </button>
      </div>
      {error ? (
        <div className="ox-error" data-testid={testId ? `${testId}-error` : undefined}>
          {error}
        </div>
      ) : null}
    </form>
  );
}

const stripStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: "8px 12px",
  borderBottom: "1px solid var(--border)",
  background: "var(--bg-2)",
};
const inputStyle: CSSProperties = {
  flex: 1,
  minWidth: 120,
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 6px",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
};
const buttonStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 8px",
  fontSize: 11,
  cursor: "pointer",
};
const primaryStyle: CSSProperties = {
  background: "var(--accent)",
  color: "#fff",
  border: "1px solid var(--accent)",
  borderRadius: 4,
  padding: "4px 10px",
  fontSize: 11,
  cursor: "pointer",
};
