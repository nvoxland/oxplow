import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import { FieldPill } from "../components/WorkItemFields.js";
import { Page } from "../tabs/Page.js";
import { useWorkListProfile } from "../useWorkListProfile.js";
import type { CanonicalState, FieldDecl, NewWorkItem, WorkItem } from "../workItems.js";

/** The states a new item may start in; every list takes them. */
const STATE_OPTIONS: Array<Extract<CanonicalState, "todo" | "blocked">> = ["todo", "blocked"];

/**
 * The list's own field values a new form starts with: what was last filed
 * (Save and Another carries it forward), for the editable fields the list
 * still declares; nothing otherwise — the list sets its own defaults.
 * Pure — exported for tests.
 */
export function fieldDefaults(fields: FieldDecl[], last: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of fields) {
    if (!f.read_only && last[f.name] !== undefined && last[f.name] !== null && last[f.name] !== "") out[f.name] = last[f.name];
  }
  return out;
}

export interface NewTaskPageProps {
  /** Defaults from the page-ref payload (a parent for "+ Item" on an epic). */
  defaults?: { parentRef?: string | null };
  /** The current list's epics, for the optional parent. */
  epics?: WorkItem[];
  /** Closes the page (caller closes the tab). */
  onClose?(): void;
  /** File the item. The page resets in place for Save and Another. */
  onSubmit(input: NewWorkItem): Promise<void>;
}

/**
 * Full-tab "New item" form, on the active work list: its title, body and
 * starting state; the list's own fields as it declares them; and a parent
 * when the list nests. Save and Another keeps the field values and the
 * parent, so a series of similar items is filed without re-picking them.
 */
export function NewTaskPage({ defaults = {}, epics = [], onClose, onSubmit }: NewTaskPageProps) {
  const profile = useWorkListProfile();
  const editable = profile.fields.filter((f) => !f.read_only);
  const [native, setNative] = useState<Record<string, unknown>>({});
  const [state, setState] = useState<"todo" | "blocked">("todo");
  const [parentRef, setParentRef] = useState<string | null>(defaults.parentRef ?? null);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const canSubmit = title.trim().length > 0 && !submitting;

  async function handleSubmit(andAnother: boolean) {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit({
        title: title.trim(),
        body: body.trim() ? body : undefined,
        ...(profile.features.hierarchy ? { parentRef } : {}),
        state,
        native: fieldDefaults(profile.fields, native),
      });
      if (andAnother) {
        setTitle("");
        setBody("");
        titleRef.current?.focus();
      } else {
        onClose?.();
      }
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Page
      testId="page-new-tasks"
      title="New item"
      kind="new-task"
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void handleSubmit(false);
        }}
        style={{ padding: "20px 24px", maxWidth: 720, display: "flex", flexDirection: "column", gap: 14 }}
      >
        <Field label="Title">
          <input
            ref={titleRef}
            data-testid="tasks-title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Title (required)"
            style={inputStyle}
          />
        </Field>
        <Field label="Description">
          <textarea
            data-testid="tasks-description"
            value={body}
            onChange={(e) => setBody(e.target.value)}
            placeholder="Description (markdown)"
            style={textareaStyle}
            rows={6}
          />
        </Field>
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          <Field label="Status">
            <select
              data-testid="tasks-status"
              value={state}
              onChange={(e) => setState(e.target.value === "blocked" ? "blocked" : "todo")}
              style={inputStyle}
            >
              {STATE_OPTIONS.map((s) => (
                <option key={s} value={s}>
                  {s === "todo" ? "Ready" : "Blocked"}
                </option>
              ))}
            </select>
          </Field>
          {editable.map((field) => (
            <Field key={field.name} label={field.title}>
              <span data-testid={`tasks-field-${field.name}`}>
                <FieldPill
                  field={field}
                  value={native[field.name]}
                  onChange={(value) => setNative((prev) => ({ ...prev, [field.name]: value }))}
                />
              </span>
            </Field>
          ))}
          {profile.features.hierarchy && epics.length > 0 ? (
            <Field label="Parent">
              <select
                data-testid="tasks-parent"
                value={parentRef ?? ""}
                onChange={(e) => setParentRef(e.target.value || null)}
                style={inputStyle}
              >
                <option value="">(none)</option>
                {epics.map((epic) => (
                  <option key={epic.ref} value={epic.ref}>
                    {epic.title}
                  </option>
                ))}
              </select>
            </Field>
          ) : null}
        </div>

        <div style={actionsRowStyle}>
          {error ? <span style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}>{error}</span> : null}
          <span style={{ flex: 1 }} />
          <button type="button" onClick={onClose} style={buttonStyle}>
            Cancel
          </button>
          <button
            type="button"
            data-testid="tasks-save-another"
            onClick={() => void handleSubmit(true)}
            disabled={!canSubmit}
            style={buttonStyle}
          >
            Save and Another
          </button>
          <button type="submit" data-testid="tasks-save" disabled={!canSubmit} style={primaryButtonStyle}>
            {submitting ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </Page>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-xs)", minWidth: 160 }}>
      <span style={{ color: "var(--text-secondary)", fontWeight: "var(--weight-medium)" }}>{label}</span>
      {children}
    </label>
  );
}

const inputStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 10px",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};

const textareaStyle: CSSProperties = {
  ...inputStyle,
  resize: "vertical",
  minHeight: 80,
  fontFamily: "inherit",
};

const buttonStyle: CSSProperties = {
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "6px 14px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  borderColor: "var(--accent)",
  color: "var(--accent-on-accent)",
};

const actionsRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  paddingTop: 12,
  borderTop: "1px solid var(--border-subtle)",
};
