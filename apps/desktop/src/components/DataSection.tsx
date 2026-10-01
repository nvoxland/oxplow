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
  approveProjectProgram,
  listDataEntities,
  providerDeclarationEffects,
  listProjectPrograms,
  listSources,
  approveSource,
  syncSource,
  subscribeOxplowEvents,
  type SourceListing,
} from "../api.js";
import type { ProjectProgram } from "../tauri-bridge/generated/bindings.js";
import { indexRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import {
  canApprove,
  entityRows,
  entitySummary,
  programRow,
  providerEffectLines,
  type EntityRowModel,
  type ProviderEffectState,
} from "./dataSectionModel.js";
import { sourceRowModel } from "./extensionRowModel.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function DataSection() {
  const nav = useOptionalPageNavigation();
  const [rows, setRows] = useState<EntityRowModel[] | null>(null);
  const [sources, setSources] = useState<SourceListing[]>([]);
  const [programs, setPrograms] = useState<ProjectProgram[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  // What approving each unapproved provider would change, by instance.
  const [effects, setEffects] = useState<Record<string, ProviderEffectState>>({});

  const refresh = useCallback(async () => {
    try {
      const [entities, listings, progs] = await Promise.all([
        listDataEntities(),
        listSources(),
        listProjectPrograms(),
      ]);
      setRows(entityRows(entities));
      setSources(listings);
      setPrograms(progs);
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

  // The unapproved providers, by name and the version on disk: their
  // diffs reload only when one of those changes, not on every refresh.
  const pendingKey = programs
    .filter((p) => p.kind === "provider" && !p.approved)
    .map((p) => `${p.name}\u0000${p.version ?? ""}`)
    .sort()
    .join("\n");
  useEffect(() => {
    let live = true;
    const pending = pendingKey === "" ? [] : pendingKey.split("\n").map((k) => k.split("\u0000")[0]!);
    setEffects(Object.fromEntries(pending.map((name) => [name, "loading" as const])));
    for (const name of pending) {
      void providerDeclarationEffects(name)
        .then((e) => {
          if (live) setEffects((prev) => ({ ...prev, [name]: e }));
        })
        .catch((e: unknown) => {
          if (live) setEffects((prev) => ({ ...prev, [name]: { error: e instanceof Error ? e.message : String(e) } }));
        });
    }
    return () => {
      live = false;
    };
  }, [pendingKey]);

  async function run(l: SourceListing) {
    const key = `${l.extension}/${l.spec.id}`;
    setBusy(key);
    try {
      // Approve & Run approves exactly the version this listing showed,
      // then runs it (`source.sync` itself never approves).
      if (!l.approved) {
        if (!l.version) throw new Error(`${key} can't be read, so it can't be approved`);
        await approveSource(l.extension, l.spec.id, l.version);
      }
      const report = await syncSource(l.extension, l.spec.id);
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

  async function approve(p: ProjectProgram) {
    const key = `${p.kind}:${p.name}`;
    setBusy(key);
    try {
      if (!p.version) throw new Error(`${p.program} can't be read, so it can't be approved`);
      setPrograms(await approveProjectProgram(p.kind, p.name, p.version));
      showToast({ message: `Approved ${p.name}. It runs from the next trigger on this machine.` });
    } catch (e) {
      recordOpError({ label: `Approve ${p.name}`, message: String(e) });
    } finally {
      setBusy(null);
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
      <h3 style={subheadStyle}>Programs</h3>
      {programs.length === 0 ? (
        <div style={mutedStyle} data-testid="data-programs-empty">
          This project&apos;s config runs no programs.
        </div>
      ) : (
        programs.map((p) => {
          const m = programRow(p);
          const effect = p.kind === "provider" && !p.approved ? effects[p.name] : undefined;
          const failed = effect !== undefined && effect !== "loading" && "error" in effect ? effect.error : null;
          const diff = effect !== undefined && effect !== "loading" && !("error" in effect) ? effect : null;
          return (
            <div key={m.key} data-testid={`program-row-${m.key}`} style={rowStyle}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span>{m.label}</span>
                <code style={{ ...mutedStyle, whiteSpace: "pre-wrap" }}>{m.command}</code>
                <span style={{ flex: 1 }} />
                <span style={m.approved ? mutedStyle : errorStyle}>{m.status}</span>
                {m.approved ? null : (
                  <button
                    type="button"
                    data-testid={`program-approve-${m.key}`}
                    title={failed !== null ? "We couldn't compare what approving would change; fix the error first" : m.approveTitle}
                    disabled={busy !== null || !canApprove(p, effect)}
                    onClick={() => void approve(p)}
                  >
                    {busy === m.key ? "Approving…" : "Approve"}
                  </button>
                )}
              </div>
              {effect === "loading" ? (
                <div style={mutedStyle}>Comparing its declarations…</div>
              ) : failed !== null ? (
                <div data-testid={`program-effects-error-${m.key}`} style={errorStyle}>
                  Couldn&apos;t compare its declarations: {failed}
                </div>
              ) : diff ? (
                <ul data-testid={`program-effects-${m.key}`} style={{ margin: "4px 0 0", paddingLeft: 18, ...mutedStyle }}>
                  {providerEffectLines(diff).map((line, i) => (
                    <li key={i}>{line}</li>
                  ))}
                </ul>
              ) : null}
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
