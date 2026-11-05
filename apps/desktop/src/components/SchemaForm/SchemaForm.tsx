/// A form from a JSON Schema (P6.B2): a command's input (the `form` page
/// component) or a provider's config (Settings → Integrations). Fields come
/// from `schemaForm.ts`.
///
/// Usability contract (.context/usability.md): Enter submits (when the form
/// has a submit), Escape cancels (a form with `onCancel`) or resets to the
/// initial value, the submit is the primary button and is disabled while a
/// field has a problem (its hover says which), and each problem shows
/// under its field. An edit form (`mode: "edit"`) saves only when changed and has a
/// Discard. A field may show only for some values of another
/// (`visible_if`) and may be secret: masked, never in a draft, blank left
/// out.

import type { CSSProperties } from "react";
import { Markdown } from "../Markdown/Markdown.js";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import {
  draftsFrom,
  fieldsOf,
  hiddenOf,
  sameDrafts,
  secretFields,
  shownFields,
  valueOf,
  type Drafts,
  type Field,
  type FieldSpec,
  type Schema,
} from "./schemaFormModel.js";

/** One choice for a field picked from a list: what it submits, what the
 *  person reads, and the heading it sits under. */
export interface FieldChoice {
  value: string;
  label: string;
  group?: string | null;
}

export interface SchemaFormProps {
  schema: Schema;
  /** The value the fields start from (and Escape returns to). */
  initial?: unknown;
  /** Every edit: the value, or null while a field has a problem. */
  onChange?(value: Record<string, unknown> | null): void;
  /** With it, the form has a submit button and Enter submits. */
  onSubmit?(value: Record<string, unknown>): void;
  /** With it, the form has a Cancel beside the submit, and Escape cancels
   *  (a create form; an edit form's Escape still puts the saved values back). */
  onCancel?(): void;
  /** Show each field's schema `description` under it (default). A command's
   *  schema is written for agents: a form that runs one passes false and
   *  says what a person reads in `fields` (`help`). */
  schemaHelp?: boolean;
  submitLabel?: string;
  busy?: boolean;
  testIdPrefix?: string;
  /** Show only these top-level fields, in this order: a name, or `{ name,
   *  visible_if, secret }`. The others keep their `initial` value and are
   *  still submitted; an unknown name is ignored. A field hidden by its
   *  `visible_if` is neither required nor submitted. */
  fields?: FieldSpec[];
  /** `edit`: it edits what is saved — Save only when changed, and a
   *  Discard. Default `create`. */
  mode?: "create" | "edit";
  /** The person's edits, kept by the host (a tab's history state): null
   *  is untouched, so the form shows `initial`. Without it the form keeps
   *  its own. A secret is never in it. */
  draft?: Drafts | null;
  /** Each edit's draft: null once it is back to `initial`. */
  onDraft?(draft: Drafts | null): void;
  /** These fields are the host's, not the person's: not drawn at all,
   *  submitted with their `initial` value. */
  fixed?: string[];
  /** A message per top-level field from the host (a rejected submit),
   *  shown under the field until the person edits it. One for a field
   *  that isn't shown appears once above the submit button. They never
   *  disable submit. */
  serverErrors?: Record<string, string>;
  /** Top-level fields picked from a list: a select of these choices
   *  instead of the field's own input. */
  choices?: Record<string, FieldChoice[]>;
  /** A second submit beside the first: the same value, sent to its own
   *  handler (Save and Another). Enter still runs the primary. */
  again?: { label: string; onSubmit(value: Record<string, unknown>): void };
}

export function SchemaForm({
  schema,
  initial,
  onChange,
  onSubmit,
  onCancel,
  schemaHelp = true,
  submitLabel,
  busy = false,
  testIdPrefix = "schema-form",
  fields: shown,
  fixed,
  serverErrors,
  choices,
  again,
  mode = "create",
  draft,
  onDraft,
}: SchemaFormProps) {
  const formRef = useRef<HTMLFormElement>(null);
  const label = submitLabel ?? (mode === "edit" ? "Save" : "Submit");
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
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const specs = useMemo(() => (shown ? shownFields(shown) : null), [shownText]);
  const listed = useMemo(() => {
    if (!specs) return fields;
    return specs.flatMap((spec) => fields.find((f) => f.path === spec.name) ?? []);
  }, [fields, specs]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const secrets = useMemo(() => secretFields(schema, specs), [specs, schemaText]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const fixedSet = useMemo(() => new Set(fixed ?? []), [fixedText]);
  // The fields edited since these server errors arrived: theirs are stale.
  const [edited, setEdited] = useState<Set<string>>(new Set());
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  useEffect(() => setEdited(new Set()), [serverText]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  const initialDrafts = useMemo(() => draftsFrom(fields, initial ?? {}), [fields, initialText]);
  const [own, setOwn] = useState<Drafts>(initialDrafts);
  const kept = draft !== undefined;
  const publicDrafts = kept ? (draft ?? initialDrafts) : own;
  // A secret's text is this form's alone: never a draft the host keeps.
  const [secretDrafts, setSecretDrafts] = useState<Drafts>({});
  const drafts = useMemo(() => ({ ...publicDrafts, ...secretDrafts }), [publicDrafts, secretDrafts]);
  // A different schema or starting value starts the drafts over — in the
  // render that sees it, not an effect after it: an effect on mount ran
  // after the person's first keystroke whenever the scheduler was behind,
  // and put the starting value back over it.
  const startKey = `${schemaText}\n${initialText}`;
  const [startedFrom, setStartedFrom] = useState(startKey);
  if (startedFrom !== startKey) {
    setStartedFrom(startKey);
    setOwn(initialDrafts);
  }
  // What is submitted: every field but those hidden by their conditions;
  // a secret is never required (blank keeps the saved one).
  const submitted = (d: Drafts) => {
    const hidden = specs ? hiddenOf(specs, d) : new Set<string>();
    return fields.filter((f) => !hidden.has(f.path)).map((f) => (secrets.has(f.path) ? { ...f, required: false } : f));
  };
  const { value, errors } = valueOf(submitted(drafts), drafts);
  const valid = Object.keys(errors).length === 0;
  const dirty = !sameDrafts(publicDrafts, initialDrafts) || Object.values(secretDrafts).some((v) => v !== "");
  const report = (next: Drafts) => {
    const out = valueOf(submitted(next), next);
    onChange?.(Object.keys(out.errors).length === 0 ? out.value : null);
  };
  const edit = (path: string, raw: string) => {
    const top = path.split(".")[0]!;
    if (serverErrors?.[top] !== undefined) setEdited((prev) => new Set(prev).add(top));
    if (secrets.has(top)) {
      const nextSecrets = { ...secretDrafts, [path]: raw };
      setSecretDrafts(nextSecrets);
      return report({ ...publicDrafts, ...nextSecrets });
    }
    const next = { ...publicDrafts, [path]: raw };
    if (!kept) setOwn(next);
    onDraft?.(sameDrafts(next, initialDrafts) ? null : next);
    report({ ...next, ...secretDrafts });
  };
  const discard = () => {
    setOwn(initialDrafts);
    setSecretDrafts({});
    onDraft?.(null);
    report(initialDrafts);
  };
  const hidden = specs ? hiddenOf(specs, drafts) : new Set<string>();
  // Nothing to pick from, or nothing to ask for, shows nothing: an
  // optional field whose choices came back empty, an object without
  // children.
  const askable = (f: Field) =>
    f.kind === "object" ? f.children.length > 0 : !(choices?.[f.path]?.length === 0 && !f.required);
  const rendered = listed.filter((f) => !hidden.has(f.path) && askable(f));
  const live = Object.entries(serverErrors ?? {}).filter(([name]) => !edited.has(name));
  const drawn = rendered.filter((f) => !fixedSet.has(f.path));
  const shownPaths = new Set(drawn.map((f) => f.path));
  const stray = live.filter(([name]) => !shownPaths.has(name));
  const spec = (path: string) => specs?.find((x) => x.name === path);
  const labelOf = (f: Field) => spec(f.path)?.label ?? f.label;
  // Why the submit is off, for its hover: the required fields still empty.
  const missing = drawn.filter((f) => f.required && !secrets.has(f.path) && errors[f.path] !== undefined && (drafts[f.path] ?? "") === "");
  const whyNot = valid
    ? ""
    : missing.length > 0
      ? `Fill in ${missing.map(labelOf).join(", ")} first`
      : `Fix ${Object.keys(errors).filter((k) => !fixedSet.has(k)).map((k) => labelOf(fields.find((f) => f.path === k) ?? ({ label: k } as Field))).join(", ")} first`;
  const firstFocus = drawn[0] && onSubmit !== undefined && mode === "create" ? `${testIdPrefix}-${drawn[0].path}` : null;
  useLayoutEffect(() => {
    if (firstFocus !== null) focusFirst(formRef.current, firstFocus);
    // Once, as the form opens (what `autoFocus` was).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const submit = () => {
    if (onSubmit && valid && !busy && (mode === "create" || dirty)) onSubmit(value);
  };
  const submitAgain = () => {
    if (again && valid && !busy) again.onSubmit(value);
  };
  return (
    <form
      ref={formRef}
      data-testid={testIdPrefix}
      style={{ display: "flex", flexDirection: "column", gap: 8 }}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          // The form's own cancel: a container's Escape mustn't also fire.
          e.stopPropagation();
          if (onCancel && mode === "create") onCancel();
          else discard();
        }
      }}
    >
      {drawn.map((f, i) => {
        const server = live.find(([name]) => name === f.path)?.[1];
        return (
          <div key={f.path} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <FieldInput
              field={f}
              label={labelOf(f)}
              help={spec(f.path)?.help ?? (schemaHelp ? f.description : null)}
              drafts={drafts}
              errors={errors}
              onEdit={edit}
              autoFocus={i === 0 && onSubmit !== undefined && mode === "create"}
              testIdPrefix={testIdPrefix}
              choices={choices?.[f.path]}
              secret={secrets.has(f.path)}
              onSubmitShortcut={submit}
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
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", paddingTop: 4 }}>
          {onCancel ? (
            <button type="button" className="ox-button" data-testid={`${testIdPrefix}-cancel`} style={buttonStyle} onClick={onCancel}>
              Cancel
            </button>
          ) : null}
          {mode === "edit" ? (
            <button type="button" className="ox-button" data-testid={`${testIdPrefix}-discard`} style={buttonStyle} disabled={!dirty || busy} onClick={discard}>
              Discard
            </button>
          ) : null}
          {again ? (
            <button type="button" className="ox-button" data-testid={`${testIdPrefix}-again`} style={buttonStyle} disabled={!valid || busy} title={whyNot} onClick={submitAgain}>
              {again.label}
            </button>
          ) : null}
          <button
            type="submit"
            className="ox-button ox-primary"
            data-testid={`${testIdPrefix}-submit`}
            style={buttonStyle}
            disabled={!valid || busy || (mode === "edit" && !dirty)}
            title={whyNot}
          >
            {busy ? `${label}…` : label}
          </button>
        </div>
      ) : null}
    </form>
  );
}

function FieldInput({
  field: f,
  label: text,
  help,
  drafts,
  errors,
  onEdit,
  autoFocus,
  testIdPrefix,
  choices,
  secret = false,
  onSubmitShortcut,
}: {
  field: Field;
  /** What the person reads over the field. */
  label: string;
  /** A line under it, for the person. */
  help: string | null;
  drafts: Drafts;
  errors: Record<string, string>;
  onEdit(path: string, raw: string): void;
  autoFocus: boolean;
  testIdPrefix: string;
  choices?: FieldChoice[];
  /** Masked: a key, a token. */
  secret?: boolean;
  onSubmitShortcut?(): void;
}) {
  const id = `${testIdPrefix}-${f.path}`;
  if (f.kind === "object") {
    return (
      <fieldset style={fieldsetStyle}>
        <legend style={labelStyle}>{text}</legend>
        {f.children.map((c) => (
          <FieldInput
            key={c.path}
            field={c}
            label={c.label}
            help={c.description}
            drafts={drafts}
            errors={errors}
            onEdit={onEdit}
            autoFocus={false}
            testIdPrefix={testIdPrefix}
          />
        ))}
      </fieldset>
    );
  }
  const raw = drafts[f.path] ?? "";
  const error = errors[f.path];
  const label = (
    <label htmlFor={id} style={labelStyle}>
      {text}
      {f.required ? <span aria-hidden> *</span> : null}
    </label>
  );
  let input;
  if (choices) {
    input = <ChoiceSelect id={id} value={raw} required={f.required} choices={choices} onChange={(v) => onEdit(f.path, v)} />;
  } else switch (f.kind) {
    case "boolean":
      // The label to the right of the box, its help beneath the label.
      return (
        <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", columnGap: 8, rowGap: 2, alignItems: "baseline" }}>
          <input
            id={id}
            data-testid={id}
            type="checkbox"
            checked={raw === "true"}
            onChange={(e) => onEdit(f.path, e.target.checked ? "true" : "false")}
            style={{ margin: 0 }}
          />
          {label}
          {help ? <span style={{ ...hintStyle, gridColumn: 2 }}>{help}</span> : null}
        </div>
      );
    case "enum":
      input = (
        <select id={id} data-testid={id} className="ox-select" value={raw} onChange={(e) => onEdit(f.path, e.target.value)}>
          <option value="">{f.required ? "Choose…" : "—"}</option>
          {f.options.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
      break;
    case "markdown":
      // The one Markdown editor: Enter adds a line, Cmd/Ctrl+Enter
      // submits (usability.md, Forms).
      input = (
        <Markdown
          value={raw}
          testId={id}
          autofocus={autoFocus}
          hidePencil
          onChange={(md) => onEdit(f.path, md)}
          onSubmitShortcut={() => onSubmitShortcut?.()}
          style={{ ...inputStyle, minHeight: 96, maxHeight: 360, overflowY: "auto" }}
        />
      );
      break;
    case "strings":
    case "json":
      input = (
        <textarea
          id={id}
          data-testid={id}
          className="ox-textarea"
          value={raw}
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
          className="ox-input"
          type={secret ? "password" : f.kind === "integer" || f.kind === "number" ? "number" : "text"}
          autoComplete={secret ? "off" : undefined}
          step={f.kind === "integer" ? 1 : undefined}
          value={raw}
          onChange={(e) => onEdit(f.path, e.target.value)}
          style={inputStyle}
        />
      );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {label}
      {input}
      {help ? <span style={hintStyle}>{help}</span> : null}
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
const buttonStyle: CSSProperties = { padding: "5px 16px" };
const inputStyle: CSSProperties = { width: "100%", boxSizing: "border-box" };
const fieldsetStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: 8,
  display: "flex",
  flexDirection: "column",
  gap: 8,
};

/** Whether the keys go to a field of some other page or overlay: a text
 *  field, a select or an editor that holds the focus outside `form`. */
function typingElsewhere(form: HTMLElement): boolean {
  const held = document.activeElement;
  if (!(held instanceof HTMLElement) || held === document.body || form.contains(held)) return false;
  return held.isContentEditable || held instanceof HTMLInputElement || held instanceof HTMLTextAreaElement || held instanceof HTMLSelectElement;
}

/** A form that opens takes the focus for its first field, unless the person
 *  is typing somewhere else: one that opens by itself (a new thread's session
 *  picker) while the launcher has the keys must not take them. */
function focusFirst(form: HTMLElement | null, id: string): void {
  if (!form || typingElsewhere(form)) return;
  const field = form.querySelector(`[id="${id}"]`);
  if (field instanceof HTMLInputElement || field instanceof HTMLSelectElement || field instanceof HTMLTextAreaElement) field.focus();
}

/** A field picked from a list: its choices, under their groups' headings
 *  in the order the groups first appear. */
function ChoiceSelect({
  id,
  value,
  required,
  choices,
  onChange,
}: {
  id: string;
  value: string;
  required: boolean;
  choices: FieldChoice[];
  onChange(value: string): void;
}) {
  const groups: { name: string | null; items: FieldChoice[] }[] = [];
  for (const c of choices) {
    const name = c.group ?? null;
    const last = groups.at(-1);
    if (last && last.name === name) last.items.push(c);
    else groups.push({ name, items: [c] });
  }
  const option = (c: FieldChoice) => (
    <option key={c.value} value={c.value}>
      {c.label}
    </option>
  );
  return (
    <select id={id} data-testid={id} className="ox-select" value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">{required ? "Choose…" : "—"}</option>
      {groups.map((g, i) =>
        g.name === null ? (
          g.items.map(option)
        ) : (
          <optgroup key={`${g.name}-${i}`} label={g.name}>
            {g.items.map(option)}
          </optgroup>
        ),
      )}
    </select>
  );
}
