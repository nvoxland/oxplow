import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import type { Task, TaskPriority, TaskStatus } from "../api.js";
import { Page } from "../tabs/Page.js";

// Edit flow now lives on TaskPage (the canonical Task page); this
// form is create-only.

// Local-only categorization choice for the form UI. The DB no longer
// stores a `kind` discriminator, but the picker still helps the user
// frame what they're filing; we collapse the choice into the title.
type TaskKind = "task" | "epic" | "subtask" | "bug" | "note";
const KIND_OPTIONS: TaskKind[] = ["task", "epic", "subtask", "bug", "note"];
const PRIORITY_OPTIONS: TaskPriority[] = ["low", "medium", "high", "urgent"];
const STATUS_OPTIONS: Array<Extract<TaskStatus, "ready" | "blocked">> = ["ready", "blocked"];

/**
 * Defaults negotiation between the original `newTaskRef` payload
 * (the form's "open with these defaults" hint) and the values the user
 * last submitted via "Save and Another". The latter wins when present
 * so a flow like "file 5 bugs at urgent priority" only requires
 * choosing `bug` / `urgent` once.
 *
 * Pure — exported for tests so we don't need a renderer to verify the
 * carry-forward logic.
 */
export function resolveSaveAndAnotherDefaults(input: {
  parentId?: number | null;
  initialCategory?: string | null;
  initialPriority?: string | null;
  lastCategory?: string | null;
  lastPriority?: string | null;
} = {}): { parentId: number | null; initialCategory: string; initialPriority: string } {
  return {
    parentId: input.parentId ?? null,
    initialCategory: input.lastCategory ?? input.initialCategory ?? "task",
    initialPriority: input.lastPriority ?? input.initialPriority ?? "medium",
  };
}

export interface NewTaskPageProps {
  /** Defaults from the page-ref payload (incl. parentId for + Task on epic). */
  defaults?: {
    parentId?: number | null;
    initialCategory?: string | null;
    initialPriority?: string | null;
  };
  /** All epics in the current thread, for the optional parent dropdown. */
  epics?: Task[];
  /** Closes the page (caller closes the tab). */
  onClose?(): void;
  /** Submit the form. The page resets in-place when `andAnother` is true. */
  onSubmit(input: {
    title: string;
    description?: string;
    parentId?: number | null;
    status?: TaskStatus;
    priority?: TaskPriority;
  }): Promise<void>;
}

/**
 * Full-tab "New tasks" form. Replaces the centred NewtasksModal
 * that used to live inside `PlanPane.tsx`. Carries Save-and-Another
 * forward by remembering the last-submitted kind/priority and
 * re-mounting the form with those values prefilled. The parent id is
 * also preserved so multiple subtasks can be filed under the same
 * epic in sequence.
 */
export function NewTaskPage({
  defaults = {},
  epics = [],
  onClose,
  onSubmit,
}: NewTaskPageProps) {
  const [lastKind, setLastKind] = useState<TaskKind | null>(null);
  const [lastPriority, setLastPriority] = useState<TaskPriority | null>(null);
  const [lastParentId, setLastParentId] = useState<number | null>(null);

  const resolved = resolveSaveAndAnotherDefaults({
    parentId: lastParentId ?? defaults.parentId,
    initialCategory: defaults.initialCategory,
    initialPriority: defaults.initialPriority,
    lastCategory: lastKind,
    lastPriority,
  });

  const [kind, setKind] = useState<TaskKind>(coerceKind(resolved.initialCategory));
  const [priority, setPriority] = useState<TaskPriority>(coercePriority(resolved.initialPriority));
  const [status, setStatus] = useState<"ready" | "blocked">("ready");
  const [parentId, setParentId] = useState<number | null>(resolved.parentId);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
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
        description: description.trim() ? description : undefined,
        parentId: parentId ?? null,
        priority,
        status,
      });
      // Save-and-Another resets the title/description fields but
      // keeps the kind/priority/parent so the user doesn't have to
      // re-pick them for a series of similar items.
      setLastKind(kind);
      setLastPriority(priority);
      setLastParentId(parentId);
      if (andAnother) {
        setTitle("");
        setDescription("");
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
      title="New task"
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
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Description (markdown)"
            style={textareaStyle}
            rows={6}
          />
        </Field>
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          <Field label="Kind">
            <select
              data-testid="tasks-kind"
              value={kind}
              onChange={(e) => setKind(coerceKind(e.target.value))}
              style={inputStyle}
            >
              {KIND_OPTIONS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Priority">
            <select
              data-testid="tasks-priority"
              value={priority}
              onChange={(e) => setPriority(coercePriority(e.target.value))}
              style={inputStyle}
            >
              {PRIORITY_OPTIONS.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Status">
            <select
              data-testid="tasks-status"
              value={status}
              onChange={(e) => setStatus(e.target.value === "blocked" ? "blocked" : "ready")}
              style={inputStyle}
            >
              {STATUS_OPTIONS.map((s) => (
                <option key={s} value={s}>
                  {s === "ready" ? "Ready" : "Blocked"}
                </option>
              ))}
            </select>
          </Field>
          {epics.length > 0 ? (
            <Field label="Parent epic">
              <select
                data-testid="tasks-parent"
                value={parentId ?? ""}
                onChange={(e) => setParentId(e.target.value ? Number(e.target.value) : null)}
                style={inputStyle}
              >
                <option value="">(none)</option>
                {epics.map((epic) => (
                  <option key={epic.id} value={epic.id}>
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
          <button
            type="submit"
            data-testid="tasks-save"
            disabled={!canSubmit}
            style={primaryButtonStyle}
          >
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

function coerceKind(input: string | null | undefined): TaskKind {
  if (input && (KIND_OPTIONS as string[]).includes(input)) return input as TaskKind;
  return "task";
}

function coercePriority(input: string | null | undefined): TaskPriority {
  if (input && (PRIORITY_OPTIONS as string[]).includes(input)) return input as TaskPriority;
  return "medium";
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
