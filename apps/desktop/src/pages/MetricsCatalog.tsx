import { useCallback, useEffect, useState } from "react";

import {
  type MetricCatalogEntry,
  listMetricCatalog,
  setMetricEnabled,
  subscribeOxplowEvents,
} from "../api.js";

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
            <td style={{ padding: "6px 8px", textAlign: "right" }}>
              {m.target == null ? "—" : m.target}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
