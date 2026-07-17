import { useState } from "react";

import { scaffoldMetric } from "../api.js";

/**
 * The "+ New metric" scaffold bar — collapsed to a button until clicked, then
 * an inline form (Enter submits, Escape cancels, key autofocuses). Lived on
 * the Metric Settings page until tsk117 folded that page away; now rides at
 * the foot of Recorded Metrics.
 */
export function NewMetricBar({
  onOpenScript,
  onCreated,
}: {
  /** Open the scaffolded script path in the editor (tsk234). */
  onOpenScript?: (path: string) => void;
  /** Reload the caller's metric list after a successful create. */
  onCreated?: () => void;
}) {
  const [creating, setCreating] = useState(false);
  const [form, setForm] = useState({
    key: "",
    title: "",
    language: "",
    glob: "**/*",
    scope: "project" as "project" | "global",
  });
  const [busy, setBusy] = useState(false);
  const [createErr, setCreateErr] = useState<string | null>(null);

  const create = async () => {
    const key = form.key.trim();
    if (!key) return;
    setBusy(true);
    setCreateErr(null);
    try {
      const path = await scaffoldMetric(
        key,
        form.title.trim() || null,
        form.language.trim() || null,
        form.glob.trim() || null,
        form.scope,
      );
      const scope = form.scope;
      setCreating(false);
      setForm({ key: "", title: "", language: "", glob: "**/*", scope: "project" });
      onCreated?.();
      // A global script lives outside the worktree; the editor opens project
      // files only, so just surface its path for global scope.
      if (scope === "project") onOpenScript?.(path);
    } catch (e) {
      setCreateErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const cancelCreate = () => {
    setCreating(false);
    setCreateErr(null);
  };

  return (
    <div style={{ marginTop: 12 }}>
      {!creating ? (
        <button
          type="button"
          onClick={() => setCreating(true)}
          data-testid="new-metric-open"
          style={{ fontSize: 12 }}
        >
          + New metric
        </button>
      ) : (
        // Enter submits, Escape cancels (usability.md). The key field autofocuses.
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              cancelCreate();
            }
          }}
          style={{
            display: "flex",
            gap: 6,
            alignItems: "center",
            flexWrap: "wrap",
            padding: 8,
            border: "1px solid var(--border, #2a2a2a)",
            borderRadius: 4,
          }}
        >
          <input
            placeholder="key (e.g. acme.todo_density)"
            value={form.key}
            onChange={(e) => setForm({ ...form, key: e.target.value })}
            data-testid="new-metric-key"
            autoFocus
            style={{ fontSize: 12, width: 200 }}
          />
          <input
            placeholder="title"
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
            style={{ fontSize: 12, width: 130 }}
          />
          <input
            placeholder="language (optional)"
            value={form.language}
            onChange={(e) => setForm({ ...form, language: e.target.value })}
            style={{ fontSize: 12, width: 120 }}
          />
          <input
            placeholder="glob"
            value={form.glob}
            onChange={(e) => setForm({ ...form, glob: e.target.value })}
            style={{ fontSize: 12, width: 90 }}
          />
          <select
            value={form.scope}
            onChange={(e) => setForm({ ...form, scope: e.target.value as "project" | "global" })}
            title="project: in this repo · global: shared across your projects"
            data-testid="new-metric-scope"
            style={{ fontSize: 12 }}
          >
            <option value="project">project</option>
            <option value="global">global</option>
          </select>
          <button
            type="submit"
            disabled={busy || !form.key.trim()}
            data-testid="new-metric-create"
            style={{ fontSize: 12 }}
          >
            Create
          </button>
          <button type="button" onClick={cancelCreate} style={{ fontSize: 12 }}>
            Cancel
          </button>
          {createErr ? <span style={{ fontSize: 11, color: "var(--err, #f85149)" }}>{createErr}</span> : null}
        </form>
      )}
    </div>
  );
}
