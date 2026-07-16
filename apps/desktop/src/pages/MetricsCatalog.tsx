import { useCallback, useEffect, useRef, useState } from "react";

import {
  type MetricCatalogEntry,
  listMetricCatalog,
  scaffoldMetric,
  setMetricEnabled,
  setMetricsEnabled,
  setMetricOverride,
  subscribeOxplowEvents,
} from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { metricRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { buildMetricSections } from "./metricCategories.js";

/** The tri-state group-checkbox state for a section: `checked` when every metric
 *  is enabled, `indeterminate` when only some are, and `nextEnabled` = what a
 *  click should apply (disable-all when fully on, else enable-all). Pure. */
export function sectionCheckboxState(entries: { enabled: boolean }[]): {
  checked: boolean;
  indeterminate: boolean;
  nextEnabled: boolean;
} {
  const allOn = entries.length > 0 && entries.every((e) => e.enabled);
  const someOn = entries.some((e) => e.enabled);
  return { checked: allOn, indeterminate: someOn && !allOn, nextEnabled: !allOn };
}

/** A checkbox that reflects/controls a whole section. HTML's `indeterminate` is a
 *  DOM property (not an attribute), so it's set via a ref. A click applies
 *  `nextEnabled` to the section. */
function GroupCheckbox({
  entries,
  disabled,
  onToggle,
  testid,
  label,
}: {
  entries: MetricCatalogEntry[];
  disabled: boolean;
  onToggle: (enabled: boolean) => void;
  testid: string;
  label: string;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const { checked, indeterminate, nextEnabled } = sectionCheckboxState(entries);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={checked}
      disabled={disabled}
      onChange={() => onToggle(nextEnabled)}
      aria-label={`${nextEnabled ? "Enable" : "Disable"} all in ${label}`}
      data-testid={testid}
    />
  );
}

/**
 * Metric Catalog (epic tsk213, P4): browse the available catalog
 * (built-in ∪ global ∪ project) and enable/disable a metric in this project —
 * the toggle writes a `use:` entry into `.oxplow/project.yaml` (or removes it) and the
 * runner reseeds. The add-and-configure home; no per-metric UI code.
 */
export function MetricsCatalog({
  onOpenScript,
  onOpenPage,
}: {
  /** Open the scaffolded script path in the editor (tsk234). */
  onOpenScript?: (path: string) => void;
  /** Navigate to a page ref — used to open a metric's detail page from its name
   *  (tsk33); the `onNavigate` fallback when there's no PageNavigationContext. */
  onOpenPage?: (ref: TabRef) => void;
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

  // Enable/disable a whole section in one config write + reseed (tsk32). Only the
  // rows that would actually change are sent; a no-op set is skipped.
  const setSection = async (entries: MetricCatalogEntry[], enabled: boolean) => {
    const keys = entries.filter((e) => e.enabled !== enabled).map((e) => e.key);
    if (keys.length === 0) return;
    setBusy("__section__");
    try {
      await setMetricsEnabled(keys, enabled);
      refresh();
    } catch (e) {
      recordOpError({
        label: `${enabled ? "Enable" : "Disable"} ${keys.length} metrics`,
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

  // One metric line: the on/off check, the metric name, and its target — nothing
  // else (no kind/scope/trigger/id). Every metric is toggleable now (tsk31); the
  // key rides as a hover title so it's still discoverable without cluttering the
  // row.
  const metricRow = (m: MetricCatalogEntry) => (
    <div
      key={m.key}
      data-testid={`catalog-row-${m.key}`}
      style={{
        display: "grid",
        gridTemplateColumns: "20px minmax(0, 1fr) auto",
        alignItems: "center",
        gap: 10,
        padding: "5px 4px",
        borderTop: "1px solid var(--border, #2a2a2a)",
      }}
    >
      <div style={{ display: "flex", justifyContent: "center" }}>
        <input
          type="checkbox"
          checked={m.enabled}
          disabled={busy != null}
          onChange={() => void toggle(m)}
          aria-label={`${m.enabled ? "Disable" : "Enable"} ${m.key}`}
          data-testid={`catalog-toggle-${m.key}`}
        />
      </div>
      <RouteLink
        to={metricRef(m.key)}
        onNavigate={onOpenPage}
        title={m.key}
        testId={`catalog-name-${m.key}`}
        style={{
          background: "none",
          border: "none",
          padding: 0,
          margin: 0,
          font: "inherit",
          fontWeight: 500,
          textAlign: "left",
          cursor: "pointer",
          color: "var(--accent, #58a6ff)",
          opacity: m.enabled ? 1 : 0.55,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          maxWidth: "100%",
        }}
      >
        {m.title}
      </RouteLink>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "flex-end" }}>
        {m.enabled ? (
          <input
            // Uncontrolled (so typing isn't clobbered mid-edit), but keyed on the
            // resolved target so an external .oxplow/project.yaml edit arriving via
            // `configChanged` remounts it with the new value instead of a stale one.
            key={`target-${m.key}-${m.target ?? "none"}`}
            type="number"
            defaultValue={m.target ?? ""}
            placeholder="target"
            disabled={busy != null}
            onBlur={(e) => {
              const raw = e.target.value.trim();
              const next = raw === "" ? null : Number(raw);
              if (next !== m.target && !(next != null && Number.isNaN(next))) {
                void override(m, { target: next });
              }
            }}
            data-testid={`catalog-target-${m.key}`}
            style={{ width: 70, fontSize: 12, textAlign: "right" }}
          />
        ) : null}
      </div>
    </div>
  );

  // Section header: the title + a tri-state group checkbox to its right. The
  // checkbox is checked when every metric is on, indeterminate when only some
  // are; clicking enables all (from off/some) or disables all (from fully-on) in
  // one batch write (tsk32).
  const sectionHeader = (title: string, testid: string, entries: MetricCatalogEntry[]) => (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        paddingBottom: 6,
        marginBottom: 4,
        borderBottom: "1px solid var(--border, #2a2a2a)",
      }}
    >
      <h2 style={{ margin: 0, fontSize: 16, fontWeight: 700 }} data-testid={testid}>
        {title}
      </h2>
      <GroupCheckbox
        entries={entries}
        disabled={busy != null}
        onToggle={(enabled) => void setSection(entries, enabled)}
        testid={`${testid}-toggle-all`}
        label={title}
      />
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
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      {buildMetricSections(
        rows,
        (r) => r.category,
        (r) => r.language,
      ).map((group) => (
        <section key={group.key} style={{ display: "flex", flexDirection: "column" }}>
          {sectionHeader(group.label, `catalog-group-${group.key}`, group.entries)}
          {group.entries.map(metricRow)}
        </section>
      ))}
      {newMetricBar}
    </div>
  );
}
