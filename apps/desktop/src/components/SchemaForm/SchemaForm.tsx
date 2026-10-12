/// A form from a JSON Schema (P6.B2): a command's input (the `form` lens
/// component) or a provider's config (Settings → Integrations). Fields come
/// from `schemaForm.ts`.
///
/// Usability contract (.context/usability.md): Enter submits (when the form
/// has a submit), Escape resets to the initial value, the submit is
/// disabled while a field has a problem, and each problem shows under its
/// field.

import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";

import { draftsFrom, fieldsOf, valueOf, type Drafts, type Field, type Schema } from "./schemaFormModel.js";

export interface SchemaFormProps {
  schema: Schema;
  /** The value the fields start from (and Escape returns to). */
  initial?: unknown;
  /** Every edit: the value, or null while a field has a problem. */
  onChange?(value: Record<string, unknown> | null): void;
  /** With it, the form has a submit button and Enter submits. */
  onSubmit?(value: Record<string, unknown>): void;
  submitLabel?: string;
  busy?: boolean;
  testIdPrefix?: string;
  /** Show only these top-level fields, in this order. The others keep
   *  their `initial` value and are still submitted; an unknown name is
   *  ignored. */
  fields?: string[];
  /** These fields show their value as text: not inputs, submitted with
   *  their `initial` value. */
  fixed?: string[];
  /** A message per top-level field from the host (a rejected submit),
   *  shown under the field until the person edits it. One for a field
   *  that isn't shown appears once above the submit button. They never
   *  disable submit. */
  serverErrors?: Record<string, string>;
}

export function SchemaForm({
  schema,
  initial,
  onChange,
  onSubmit,
  submitLabel = "Submit",
  busy = false,
  testIdPrefix = "schema-form",
  fields: shown,
  fixed,
  serverErrors,
}: SchemaFormProps) {
  // Keyed on content: a host re-reading its data passes fresh copies of
  // the same schema and starting value, and that mustn't throw away what
  // the person typed (tsk1054). A different starting value does.
  const schemaText = JSON.stringify(schema);
  const initialText = JSON.stringify(initial ?? {});
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const fields = useMemo(() => fieldsOf(schema), [schemaText]);
  const shownText = JSON.stringify(shown ?? null);
  const fixedText = JSON.stringify(fixed ?? []);
  const serverText = JSON.stringify(serverErrors ?? {});
  const rendered = useMemo(() => {
    if (!shown) return fields;
    return shown.flatMap((name) => fields.find((f) => f.path === name) ?? []);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  }, [fields, shownText]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const fixedSet = useMemo(() => new Set(fixed ?? []), [fixedText]);
  // The fields edited since these server errors arrived: theirs are stale.
  const [edited, setEdited] = useState<Set<string>>(new Set());
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  useEffect(() => setEdited(new Set()), [serverText]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const initialDrafts = useMemo(() => draftsFrom(fields, initial ?? {}), [fields, initialText]);
  const [drafts, setDrafts] = useState<Drafts>(initialDrafts);
  useEffect(() => setDrafts(initialDrafts), [initialDrafts]);
  const { value, errors } = valueOf(fields, drafts);
  const valid = Object.keys(errors).length === 0;
  const edit = (path: string, raw: string) => {
    const top = path.split(".")[0]!;
    if (serverErrors?.[top] !== undefined) setEdited((prev) => new Set(prev).add(top));
    const next = { ...drafts, [path]: raw };
    setDrafts(next);
    const out = valueOf(fields, next);
    onChange?.(Object.keys(out.errors).length === 0 ? out.value : null);
  };
  const live = Object.entries(serverErrors ?? {}).filter(([name]) => !edited.has(name));
  const shownPaths = new Set(rendered.map((f) => f.path));
  const stray = live.filter(([name]) => !shownPaths.has(name));
  const firstEditable = rendered.findIndex((f) => !fixedSet.has(f.path));
  return (
    <form
      data-testid={testIdPrefix}
      style={{ display: "flex", flexDirection: "column", gap: 8 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (onSubmit && valid && !busy) onSubmit(value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          // The form's own cancel: a container's Escape mustn't also fire.
          e.stopPropagation();
          setDrafts(initialDrafts);
          const out = valueOf(fields, initialDrafts);
          onChange?.(Object.keys(out.errors).length === 0 ? out.value : null);
        }
      }}
    >
      {rendered.map((f, i) => {
        const server = live.find(([name]) => name === f.path)?.[1];
        return (
          <div key={f.path} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <FieldInput
              field={f}
              drafts={drafts}
              errors={errors}
              onEdit={edit}
              autoFocus={i === firstEditable && onSubmit !== undefined}
              testIdPrefix={testIdPrefix}
              fixed={fixedSet.has(f.path)}
            />
            {server !== undefined && errors[f.path] === undefined ? (
              <span data-testid={`${testIdPrefix}-${f.path}-error`} style={errorStyle}>
                {server}
              </span>
            ) : null}
          </div>
        );
      })}
      {stray.length > 0 ? (
        <div data-testid={`${testIdPrefix}-form-error`} style={errorStyle}>
          {stray.map(([name, message]) => (
            <div key={name}>{message}</div>
          ))}
        </div>
      ) : null}
      {onSubmit ? (
        <div>
          <button type="submit" data-testid={`${testIdPrefix}-submit`} disabled={!valid || busy}>
            {busy ? `${submitLabel}…` : submitLabel}
          </button>
        </div>
      ) : null}
    </form>
  );
}

function FieldInput({
  field: f,
  drafts,
  errors,
  onEdit,
  autoFocus,
  testIdPrefix,
  fixed,
}: {
  field: Field;
  drafts: Drafts;
  errors: Record<string, string>;
  onEdit(path: string, raw: string): void;
  autoFocus: boolean;
  testIdPrefix: string;
  fixed: boolean;
}) {
  const id = `${testIdPrefix}-${f.path}`;
  if (f.kind === "object") {
    return (
      <fieldset style={fieldsetStyle}>
        <legend style={labelStyle}>{f.label}</legend>
        {f.children.map((c) => (
          <FieldInput
            key={c.path}
            field={c}
            drafts={drafts}
            errors={errors}
            onEdit={onEdit}
            autoFocus={false}
            testIdPrefix={testIdPrefix}
            fixed={fixed}
          />
        ))}
      </fieldset>
    );
  }
  const raw = drafts[f.path] ?? "";
  if (fixed) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <span style={labelStyle}>{f.label}</span>
        <span data-testid={`${id}-fixed`} style={{ fontFamily: f.kind === "text" ? undefined : "var(--font-mono)", whiteSpace: "pre-wrap" }}>
          {raw}
        </span>
      </div>
    );
  }
  const error = errors[f.path];
  const label = (
    <label htmlFor={id} style={labelStyle}>
      {f.label}
      {f.required ? <span aria-hidden> *</span> : null}
    </label>
  );
  let input;
  switch (f.kind) {
    case "boolean":
      return (
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <input
            id={id}
            data-testid={id}
            type="checkbox"
            checked={raw === "true"}
            autoFocus={autoFocus}
            onChange={(e) => onEdit(f.path, e.target.checked ? "true" : "false")}
          />
          {label}
          {f.description ? <span style={hintStyle}>{f.description}</span> : null}
        </div>
      );
    case "enum":
      input = (
        <select id={id} data-testid={id} value={raw} autoFocus={autoFocus} onChange={(e) => onEdit(f.path, e.target.value)}>
          <option value="">{f.required ? "Choose…" : "—"}</option>
          {f.options.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
      break;
    case "strings":
    case "json":
      input = (
        <textarea
          id={id}
          data-testid={id}
          value={raw}
          autoFocus={autoFocus}
          rows={Math.min(8, Math.max(2, raw.split("\n").length))}
          placeholder={f.kind === "strings" ? "One per line" : "JSON"}
          onChange={(e) => onEdit(f.path, e.target.value)}
          style={{ ...inputStyle, fontFamily: "var(--font-mono)" }}
        />
      );
      break;
    default:
      input = (
        <input
          id={id}
          data-testid={id}
          type={f.kind === "integer" || f.kind === "number" ? "number" : "text"}
          step={f.kind === "integer" ? 1 : undefined}
          value={raw}
          autoFocus={autoFocus}
          onChange={(e) => onEdit(f.path, e.target.value)}
          style={inputStyle}
        />
      );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {label}
      {input}
      {f.description ? <span style={hintStyle}>{f.description}</span> : null}
      {error && raw !== "" ? (
        <span data-testid={`${id}-error`} style={errorStyle}>
          {error}
        </span>
      ) : null}
    </div>
  );
}

const labelStyle: CSSProperties = { fontSize: "var(--text-sm)", fontWeight: 600 };
const hintStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const inputStyle: CSSProperties = { width: "100%", boxSizing: "border-box" };
const fieldsetStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: 8,
  display: "flex",
  flexDirection: "column",
  gap: 8,
};
