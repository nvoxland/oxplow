import { useCallback, useEffect, useState } from "react";

import {
  type MetricCatalogEntry,
  listMetricCatalog,
  scaffoldMetric,
  setMetricEnabled,
  setMetricOverride,
  subscribeOxplowEvents,
} from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { categoryLabel, groupByCategory } from "./metricCategories.js";


/** Group catalog entries by category in display order. Thin wrapper over the
 *  shared `groupByCategory` (used by the Recorded Metrics page too). */
export function groupCatalog(
  rows: MetricCatalogEntry[],
): Array<{ category: string | null; entries: MetricCatalogEntry[] }> {
  return groupByCategory(rows, (r) => r.category);
}

/**
 * Metric Catalog (epic tsk213, P4): browse the available catalog
 * (built-in ∪ global ∪ project) and enable/disable a metric in this project —
 * the toggle writes a `use:` entry into `.oxplow/project.yaml` (or removes it) and the
 * runner reseeds. The add-and-configure home; no per-metric UI code.
 */
export function MetricsCatalog({
  onOpenScript,
}: {
  /** Open the scaffolded script path in the editor (tsk234). */
  onOpenScript?: (path: string) => void;
} = {}) {
  const [rows, setRows] = useState<MetricCatalogEntry[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [form, setForm] = useState({
    key: "",
    title: "",
    language: "",
    glob: "**/*",
    scope: "project" as "project" | "global",
  });
  const [createErr, setCreateErr] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void listMetricCatalog().then(setRows);
  }, []);

  useEffect(() => {
    refresh();
    // Config edits (incl. our own toggle) re-resolve the catalog.
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "configChanged" || e.kind === "metricSamplesChanged") refresh();
    });
    return off;
  }, [refresh]);

  const toggle = async (entry: MetricCatalogEntry) => {
    setBusy(entry.key);
    try {
      await setMetricEnabled(entry.key, !entry.enabled);
      refresh();
    } catch (e) {
      // Surface instead of silently reverting the checkbox on refresh.
      recordOpError({
        label: `${entry.enabled ? "Disable" : "Enable"} ${entry.key}`,
        message: e instanceof Error ? e.message : String(e),
      });
      refresh();
    } finally {
      setBusy(null);
    }
  };

  // Write a target override into .oxplow/project.yaml (tsk233). An empty target clears
  // it. `trigger` is inherent to the definition — not overridable (tsk290).
  const override = async (entry: MetricCatalogEntry, next: { target: number | null }) => {
    setBusy(entry.key);
    try {
      await setMetricOverride(entry.key, next.target);
      refresh();
    } catch (e) {
      recordOpError({
        label: `Update ${entry.key}`,
        message: e instanceof Error ? e.message : String(e),
      });
      refresh();
    } finally {
      setBusy(null);
    }
  };

  const create = async () => {
    const key = form.key.trim();
    if (!key) return;
    setBusy(key);
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
      refresh();
      // A global script lives outside the worktree; the editor opens project
      // files only, so just surface its path for global scope.
      if (scope === "project") onOpenScript?.(path);
    } catch (e) {
      setCreateErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const cancelCreate = () => {
    setCreating(false);
    setCreateErr(null);
  };

  const newMetricBar = (
    <div style={{ marginTop: 12 }}>
      {!creating ? (
        <button
          type="button"
          onClick={() => setCreating(true)}
          data-testid="catalog-new-metric"
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
            disabled={busy != null || !form.key.trim()}
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

  if (rows.length === 0) {
    return (
      <div>
        <div style={{ opacity: 0.6 }}>No metrics in the catalog.</div>
        {newMetricBar}
      </div>
    );
  }

  return (
    <div>
    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
      <thead>
        <tr style={{ textAlign: "left", opacity: 0.6 }}>
          <th style={{ padding: "4px 8px" }}>Enabled</th>
          <th style={{ padding: "4px 8px" }}>Metric</th>
          <th style={{ padding: "4px 8px" }}>Kind</th>
          <th style={{ padding: "4px 8px" }}>Language</th>
          <th style={{ padding: "4px 8px" }}>Scope</th>
          <th style={{ padding: "4px 8px" }}>Trigger</th>
          <th style={{ padding: "4px 8px", textAlign: "right" }}>Target</th>
        </tr>
      </thead>
      {groupCatalog(rows).map((group) => (
        <tbody key={group.category ?? "other"}>
          <tr>
            <td colSpan={7} style={{ padding: "22px 8px 6px" }} data-testid={`catalog-group-${group.category ?? "other"}`}>
              <h2
                style={{
                  margin: 0,
                  fontSize: 17,
                  fontWeight: 700,
                  paddingBottom: 6,
                  borderBottom: "1px solid var(--border, #2a2a2a)",
                }}
              >
                {categoryLabel(group.category)}
              </h2>
            </td>
          </tr>
          {group.entries.map((m) => {
            // Only toggleable metrics expose enable/disable + target/trigger
            // overrides; always-on producers/plugins are read-only.
            const editable = m.toggleable && m.enabled;
            return (
              <tr key={m.key} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
                <td style={{ padding: "6px 8px" }}>
                  {m.toggleable ? (
                    <input
                      type="checkbox"
                      checked={m.enabled}
                      disabled={busy === m.key}
                      onChange={() => void toggle(m)}
                      aria-label={`${m.enabled ? "Disable" : "Enable"} ${m.key}`}
                      data-testid={`catalog-toggle-${m.key}`}
                    />
                  ) : (
                    <span
                      title="Always on — recorded automatically by a producer; nothing to enable."
                      style={{
                        fontSize: 10,
                        padding: "1px 6px",
                        borderRadius: 4,
                        background: "var(--surface-2, #1c1c1c)",
                        opacity: 0.7,
                        whiteSpace: "nowrap",
                      }}
                      data-testid={`catalog-alwayson-${m.key}`}
                    >
                      Always on
                    </span>
                  )}
                </td>
                <td style={{ padding: "6px 8px" }}>
                  <div style={{ fontWeight: 600 }}>{m.title}</div>
                  <div style={{ opacity: 0.5, fontFamily: "monospace", fontSize: 11 }}>{m.key}</div>
                </td>
                <td style={{ padding: "6px 8px" }}>{m.kind}</td>
                <td style={{ padding: "6px 8px" }}>{m.language ?? "—"}</td>
                <td style={{ padding: "6px 8px" }}>
                  <span
                    style={{
                      fontSize: 11,
                      padding: "1px 6px",
                      borderRadius: 4,
                      background: "var(--surface-2, #1c1c1c)",
                      opacity: 0.8,
                    }}
                  >
                    {m.scope}
                  </span>
                </td>
                <td style={{ padding: "6px 8px" }}>
                  {/* Trigger is inherent to the definition — read-only, never
                      user-picked (tsk290). */}
                  <span style={{ opacity: 0.5 }} data-testid={`catalog-trigger-${m.key}`}>
                    {m.trigger}
                  </span>
                </td>
                <td style={{ padding: "6px 8px", textAlign: "right" }}>
                  {editable ? (
                    <input
                      // Uncontrolled (so typing isn't clobbered mid-edit), but keyed
                      // on the resolved target so an external .oxplow/project.yaml edit
                      // arriving via `configChanged` remounts it with the new value
                      // instead of showing a stale one.
                      key={`target-${m.key}-${m.target ?? "none"}`}
                      type="number"
                      defaultValue={m.target ?? ""}
                      disabled={busy === m.key}
                      onBlur={(e) => {
                        const raw = e.target.value.trim();
                        const next = raw === "" ? null : Number(raw);
                        if (next !== m.target && !(next != null && Number.isNaN(next))) {
                          void override(m, { target: next });
                        }
                      }}
                      data-testid={`catalog-target-${m.key}`}
                      style={{ width: 64, fontSize: 12, textAlign: "right" }}
                    />
                  ) : m.target == null ? (
                    "—"
                  ) : (
                    m.target
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      ))}
    </table>
    {newMetricBar}
    </div>
  );
}
