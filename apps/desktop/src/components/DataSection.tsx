/// "Data" section body for SettingsPage: every semantic-layer entity with
/// its provider and row count, and every extension source with its last
/// sync and a Run button (Approve & Run for an exec source nobody on this
/// machine approved yet). Credentials and enabling stay under Extensions.
/// See `.context/semantic-layer.md`.
///
/// Usability contract (.context/usability.md): no modals; failures land in
/// opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  describeSchema,
  listSources,
  runSource,
  semanticRowCounts,
  subscribeOxplowEvents,
  type SourceListing,
} from "../api.js";
import { indexRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { entityRows, entitySummary, type EntityRowModel } from "./dataSectionModel.js";
import { sourceRowModel } from "./extensionRowModel.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function DataSection() {
  const nav = useOptionalPageNavigation();
  const [rows, setRows] = useState<EntityRowModel[] | null>(null);
  const [sources, setSources] = useState<SourceListing[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [schema, counts, listings] = await Promise.all([describeSchema(), semanticRowCounts(), listSources()]);
      setRows(entityRows(schema, counts));
      setSources(listings);
    } catch (e) {
      recordOpError({ label: "List data", message: String(e) });
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Scheduled and agent-triggered runs land here too.
    return subscribeOxplowEvents((event) => {
      if (event.kind === "sourceSynced") void refresh();
    });
  }, [refresh]);

  async function run(l: SourceListing) {
    const key = `${l.extension}/${l.spec.id}`;
    setBusy(key);
    try {
      const report = await runSource(l.extension, l.spec.id, !l.approved);
      const counts = Object.entries(report.rowCounts)
        .map(([e, n]) => `${n} ${e}`)
        .join(", ");
      showToast({ message: `Synced ${key}: ${counts || "no rows"}.` });
    } catch (e) {
      recordOpError({ label: `Run source ${key}`, message: String(e) });
    } finally {
      setBusy(null);
      await refresh();
    }
  }

  if (rows === null) return <div style={mutedStyle}>Loading…</div>;
  return (
    <div data-testid="data-section">
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <span style={mutedStyle} data-testid="data-summary">
          {entitySummary(rows)}
        </span>
        <span style={{ flex: 1 }} />
        {nav ? (
          <button type="button" data-testid="data-explore" onClick={() => nav.navigate(indexRef("explore-data"))}>
            Explore Data
          </button>
        ) : null}
      </div>
      <table style={tableStyle}>
        <thead>
          <tr>
            <th style={thStyle}>Entity</th>
            <th style={thStyle}>From</th>
            <th style={{ ...thStyle, textAlign: "right" }}>Rows</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name} data-testid={`data-entity-${r.name}`} title={r.description}>
              <td style={tdStyle}>
                <code>{r.name}</code>
              </td>
              <td style={tdStyle}>{r.owner}</td>
              <td style={{ ...tdStyle, textAlign: "right", color: r.available ? undefined : "var(--text-secondary)" }}>
                {r.rows}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3 style={subheadStyle}>Sources</h3>
      {sources.length === 0 ? (
        <div style={mutedStyle} data-testid="data-sources-empty">
          No sources. An extension can declare one to bring in outside data.
        </div>
      ) : (
        sources.map((l) => {
          const s = sourceRowModel(l);
          const key = `${l.extension}/${s.id}`;
          return (
            <div key={key} data-testid={`source-row-${key}`} style={rowStyle}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span>
                  <code>{key}</code>
                </span>
                <span style={mutedStyle}>
                  {l.spec.runtime} · {s.schedule} · {s.status}
                  {s.lastRunAt ? ` · last run ${new Date(s.lastRunAt).toLocaleString()}` : ""}
                </span>
                <span style={{ flex: 1 }} />
                <button
                  type="button"
                  data-testid={`source-run-${key}`}
                  title={s.actionTitle}
                  disabled={busy !== null}
                  onClick={() => void run(l)}
                >
                  {busy === key ? "Running…" : s.actionLabel}
                </button>
              </div>
              {s.error ? <div style={errorStyle}>{s.error}</div> : null}
              {s.missingCredentials ? <div style={mutedStyle}>{s.missingCredentials}</div> : null}
            </div>
          );
        })
      )}
    </div>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const tableStyle: CSSProperties = { width: "100%", borderCollapse: "collapse", fontSize: "var(--text-sm)" };
const thStyle: CSSProperties = {
  textAlign: "left",
  fontWeight: 600,
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
  padding: "4px 6px",
  borderBottom: "1px solid var(--border-subtle)",
};
const tdStyle: CSSProperties = { padding: "3px 6px", borderBottom: "1px solid var(--border-subtle)" };
const subheadStyle: CSSProperties = { fontSize: "var(--text-sm)", margin: "16px 0 4px" };
const rowStyle: CSSProperties = { padding: "6px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
