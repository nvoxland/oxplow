import { useCallback, useEffect, useState } from "react";

import {
  type MetricCatalogEntry,
  listMetricCatalog,
  setMetricEnabled,
  setMetricOverride,
  subscribeOxplowEvents,
} from "../api.js";

const TRIGGERS = ["on-snapshot", "on-effort-complete", "manual"] as const;

/**
 * Metric Catalog (epic tsk213, P4): browse the available catalog
 * (built-in ∪ global ∪ project) and enable/disable a metric in this project —
 * the toggle writes a `use:` entry into `oxplow.yaml` (or removes it) and the
 * runner reseeds. The add-and-configure home; no per-metric UI code.
 */
export function MetricsCatalog() {
  const [rows, setRows] = useState<MetricCatalogEntry[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

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
    } finally {
      setBusy(null);
    }
  };

  // Write a target/trigger override into oxplow.yaml (tsk233). Preserves the
  // sibling field at its current resolved value; an empty target clears it.
  const override = async (
    entry: MetricCatalogEntry,
    next: { target?: number | null; trigger?: string },
  ) => {
    setBusy(entry.key);
    try {
      const target = next.target !== undefined ? next.target : entry.target;
      const trigger = next.trigger !== undefined ? next.trigger : entry.trigger;
      await setMetricOverride(entry.key, target, trigger);
      refresh();
    } finally {
      setBusy(null);
    }
  };

  if (rows.length === 0) {
    return <div style={{ opacity: 0.6 }}>No metrics in the catalog.</div>;
  }

  return (
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
      <tbody>
        {rows.map((m) => (
          <tr key={m.key} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
            <td style={{ padding: "6px 8px" }}>
              <input
                type="checkbox"
                checked={m.enabled}
                disabled={busy === m.key}
                onChange={() => void toggle(m)}
                aria-label={`${m.enabled ? "Disable" : "Enable"} ${m.key}`}
                data-testid={`catalog-toggle-${m.key}`}
              />
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
              {m.enabled ? (
                <select
                  value={TRIGGERS.includes(m.trigger as (typeof TRIGGERS)[number]) ? m.trigger : ""}
                  disabled={busy === m.key}
                  onChange={(e) => void override(m, { trigger: e.target.value })}
                  data-testid={`catalog-trigger-${m.key}`}
                  style={{ fontSize: 12 }}
                >
                  {!TRIGGERS.includes(m.trigger as (typeof TRIGGERS)[number]) ? (
                    <option value="">{m.trigger}</option>
                  ) : null}
                  {TRIGGERS.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </select>
              ) : (
                <span style={{ opacity: 0.5 }}>{m.trigger}</span>
              )}
            </td>
            <td style={{ padding: "6px 8px", textAlign: "right" }}>
              {m.enabled ? (
                <input
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
        ))}
      </tbody>
    </table>
  );
}
