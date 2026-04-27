import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import type {
  WorkItem,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "../api.js";
import { Page } from "../tabs/Page.js";

const KIND_OPTIONS: WorkItemKind[] = ["task", "epic", "subtask", "bug", "note"];
const PRIORITY_OPTIONS: WorkItemPriority[] = ["low", "medium", "high", "urgent"];

/**
 * Defaults negotiation between the original `newWorkItemRef` payload
 * (the form's "open with these defaults" hint) and the values the user
 * last submitted via "Save and Another". The latter wins when present
 * so a flow like "file 5 bugs at urgent priority" only requires
 * choosing `bug` / `urgent` once.
 *
 * Pure — exported for tests so we don't need a renderer to verify the
 * carry-forward logic.
 */
export function resolveSaveAndAnotherDefaults(input: {
  parentId?: string | null;
  initialCategory?: string | null;
  initialPriority?: string | null;
  lastCategory?: string | null;
  lastPriority?: string | null;
} = {}): { parentId: string | null; initialCategory: string; initialPriority: string } {
  return {
    parentId: input.parentId ?? null,
    initialCategory: input.lastCategory ?? input.initialCategory ?? "task",
    initialPriority: input.lastPriority ?? input.initialPriority ?? "medium",
  };
}

export interface NewWorkItemPageProps {
  /** Defaults from the page-ref payload (incl. parentId for + Task on epic). */
  defaults?: {
    parentId?: string | null;
    initialCategory?: string | null;
    initialPriority?: string | null;
  };
  /** All epics in the current thread, for the optional parent dropdown. */
  epics?: WorkItem[];
  /** When present the page renders in edit mode: form is prefilled from
   *  the item, title becomes "Edit work item", and submit calls onUpdate
   *  instead of onSubmit. The status field is exposed only in edit mode. */
  editingItem?: WorkItem | null;
  /** Closes the page (caller closes the tab). */
  onClose?(): void;
  /** Submit the form. The page resets in-place when `andAnother` is true.
   *  Only invoked in create mode. */
  onSubmit(input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    parentId?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  }): Promise<void>;
  /** Update an existing item. Required when `editingItem` is set. */
  onUpdate?(itemId: string, changes: {
    title?: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  }): Promise<void>;
}

/**
 * Full-tab "New work item" form. Replaces the centred NewWorkItemModal
 * that used to live inside `PlanPane.tsx`. Carries Save-and-Another
 * forward by remembering the last-submitted kind/priority and
 * re-mounting the form with those values prefilled. The parent id is
 * also preserved so multiple subtasks can be filed under the same
 * epic in sequence.
 */
export function NewWorkItemPage({
  defaults = {},
  epics = [],
  editingItem = null,
  onClose,
  onSubmit,
  onUpdate,
}: NewWorkItemPageProps) {
  const isEditing = !!editingItem;
  const [lastKind, setLastKind] = useState<WorkItemKind | null>(null);
  const [lastPriority, setLastPriority] = useState<WorkItemPriority | null>(null);
  const [lastParentId, setLastParentId] = useState<string | null>(null);

  const resolved = resolveSaveAndAnotherDefaults({
    parentId: lastParentId ?? defaults.parentId,
    initialCategory: defaults.initialCategory,
    initialPriority: defaults.initialPriority,
    lastCategory: lastKind,
    lastPriority,
  });

  const [kind, setKind] = useState<WorkItemKind>(
    isEditing ? editingItem.kind : coerceKind(resolved.initialCategory),
  );
  const [priority, setPriority] = useState<WorkItemPriority>(
    isEditing ? editingItem.priority : coercePriority(resolved.initialPriority),
  );
  const [parentId, setParentId] = useState<string | null>(
    isEditing ? editingItem.parent_id : resolved.parentId,
  );
  const [status, setStatus] = useState<WorkItemStatus>(
    isEditing ? editingItem.status : "ready",
  );
  const [title, setTitle] = useState(isEditing ? editingItem.title : "");
  const [description, setDescription] = useState(isEditing ? editingItem.description ?? "" : "");
  const [acceptance, setAcceptance] = useState(isEditing ? editingItem.acceptance_criteria ?? "" : "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleRef = useRef<HTMLInputElement | null>(null);

  // When the host swaps `editingItem` to a new id (e.g. user opened a
  // different edit tab while this one was already mounted, or the item
  // was refreshed from a subscription), repopulate the form fields.
  // Strict-equality check so a no-op re-render doesn't blow away a
  // user's mid-edit text.
  const editingItemId = editingItem?.id ?? null;
  useEffect(() => {
    if (!editingItem) return;
    setTitle(editingItem.title);
    setDescription(editingItem.description ?? "");
    setAcceptance(editingItem.acceptance_criteria ?? "");
    setKind(editingItem.kind);
    setPriority(editingItem.priority);
    setStatus(editingItem.status);
    setParentId(editingItem.parent_id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editingItemId]);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const canSubmit = title.trim().length > 0 && !submitting;

  async function handleSubmit(andAnother: boolean) {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      if (isEditing && editingItem) {
        if (!onUpdate) throw new Error("onUpdate required for edit mode");
        await onUpdate(editingItem.id, {
          title: title.trim(),
          description,
          acceptanceCriteria: acceptance.trim() ? acceptance : null,
          status,
          priority,
        });
        onClose?.();
        return;
      }
      await onSubmit({
        kind,
        title: title.trim(),
        description: description.trim() ? description : undefined,
        acceptanceCriteria: acceptance.trim() ? acceptance : null,
        parentId: parentId ?? null,
        priority,
      });
      // Save-and-Another resets the title/description/acceptance fields
      // but keeps the kind/priority/parent so the user doesn't have to
      // re-pick them for a series of similar items.
      setLastKind(kind);
      setLastPriority(priority);
      setLastParentId(parentId);
      if (andAnother) {
        setTitle("");
        setDescription("");
        setAcceptance("");
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
      testId="page-new-work-item"
      title={isEditing ? "Edit work item" : "New work item"}
      kind={isEditing ? "edit work item" : "new work item"}
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
            data-testid="work-item-title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Title (required)"
            style={inputStyle}
          />
        </Field>
        <Field label="Description">
          <textarea
            data-testid="work-item-description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Description"
            style={textareaStyle}
            rows={5}
          />
        </Field>
        <Field label="Acceptance criteria">
          <textarea
            data-testid="work-item-acceptance"
            value={acceptance}
            onChange={(e) => setAcceptance(e.target.value)}
            placeholder="One per line"
            style={textareaStyle}
            rows={3}
          />
        </Field>
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          <Field label="Kind">
            <select
              data-testid="work-item-kind"
              value={kind}
              onChange={(e) => setKind(coerceKind(e.target.value))}
              style={inputStyle}
              disabled={isEditing}
            >
              {KIND_OPTIONS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </Field>
          {isEditing ? (
            <Field label="Status">
              <select
                data-testid="work-item-status"
                value={status}
                onChange={(e) => setStatus(e.target.value as WorkItemStatus)}
                style={inputStyle}
              >
                <option value="ready">ready</option>
                {editingItem?.status === "in_progress" ? (
                  <option value="in_progress">in_progress</option>
                ) : null}
                <option value="blocked">blocked</option>
                <option value="human_check">human_check</option>
                <option value="done">done</option>
                <option value="archived">archived</option>
                <option value="canceled">canceled</option>
              </select>
            </Field>
          ) : null}
          <Field label="Priority">
            <select
              data-testid="work-item-priority"
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
          {epics.length > 0 ? (
            <Field label="Parent epic">
              <select
                data-testid="work-item-parent"
                value={parentId ?? ""}
                onChange={(e) => setParentId(e.target.value || null)}
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
          {error ? <span style={{ color: "var(--severity-critical)", fontSize: 12 }}>{error}</span> : null}
          <span style={{ flex: 1 }} />
          <button type="button" onClick={onClose} style={buttonStyle}>
            Cancel
          </button>
          {!isEditing ? (
            <button
              type="button"
              data-testid="work-item-save-another"
              onClick={() => void handleSubmit(true)}
              disabled={!canSubmit}
              style={buttonStyle}
            >
              Save and Another
            </button>
          ) : null}
          <button
            type="submit"
            data-testid="work-item-save"
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
    <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12, minWidth: 160 }}>
      <span style={{ color: "var(--text-secondary)", fontWeight: 500 }}>{label}</span>
      {children}
    </label>
  );
}

function coerceKind(input: string | null | undefined): WorkItemKind {
  if (input && (KIND_OPTIONS as string[]).includes(input)) return input as WorkItemKind;
  return "task";
}

function coercePriority(input: string | null | undefined): WorkItemPriority {
  if (input && (PRIORITY_OPTIONS as string[]).includes(input)) return input as WorkItemPriority;
  return "medium";
}

const inputStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 10px",
  fontFamily: "inherit",
  fontSize: 13,
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
  fontSize: 13,
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
