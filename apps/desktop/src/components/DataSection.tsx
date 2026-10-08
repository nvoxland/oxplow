/// "Data" section body for SettingsPage: every semantic-layer entity with
/// its owner (core or an extension) and row count (counted per model after the list shows), and every extension source with its last
/// sync and a Run button (Approve & Run for an exec source, or a Starlark
/// one that calls a model, nobody on this machine approved yet). Credentials and enabling stay under Extensions.
/// Delivery lists the events a consumer couldn't take, with Retry and
/// Discard (confirmed inline). See `.context/semantic-layer.md`.
///
/// Usability contract (.context/usability.md): no modals; failures land in
/// opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useId, useRef, useState } from "react";

import {
  approveProjectProgram,
  programSource,
  listDataEntities,
  providerDeclarationImpact,
  listProjectPrograms,
  listCollectors,
  approveCollector,
  querySql,
  runCommand,
  syncCollector,
  subscribeOxplowEvents,
  type CollectorListing,
} from "../api.js";
import type { DataEntity, ProjectProgram } from "../tauri-bridge/generated/bindings.js";
import { indexRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import {
  backfillAsk, backfillRunLabel,
  backfillDone,
  canApprove,
  entityRows,
  entitySummary,
  programRow,
  providerImpactLines,
  type BackfillResult,
  type EntityCount,
  type ProviderImpactState,
} from "./dataSectionModel.js";
import { collectorRan, collectorRowModel } from "./extensionRowModel.js";
import { DeliveryList } from "./DeliveryList.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function DataSection() {
  const nav = useOptionalPageNavigation();
  const [entities, setEntities] = useState<DataEntity[] | null>(null);
  const [counts, setCounts] = useState<Record<string, EntityCount>>({});
  // Each refresh recounts; a newer one stops an older one's counting.
  const counting = useRef(0);
  const [collectors, setCollectors] = useState<CollectorListing[]>([]);
  const [programs, setPrograms] = useState<ProjectProgram[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  // What approving each unapproved provider would change, by instance.
  const [impacts, setImpacts] = useState<Record<string, ProviderImpactState>>({});

  const refresh = useCallback(async () => {
    try {
      const [entities, listings, progs] = await Promise.all([
        listDataEntities(),
        listCollectors(),
        listProjectPrograms(),
      ]);
      setEntities(entities);
      setCollectors(listings);
      setPrograms(progs);
      void countRows(entities);
    } catch (e) {
      recordOpError({ label: "List data", message: String(e) });
      setEntities([]);
    }
  }, []);

  // One model at a time, so counting never crowds the daemon: a model too
  // big to count within query_sql's timeout costs its own cell (tsk1065).
  async function countRows(entities: DataEntity[]) {
    const run = ++counting.current;
    setCounts({});
    for (const e of entities.filter((e) => e.kind !== "declared")) {
      let count: EntityCount;
      try {
        // Names come from the registry, never from user input.
        const result = await querySql(`SELECT count(*) FROM "${e.name}"`, [], 1);
        const n = result.rows[0]?.[0];
        count = typeof n === "number" ? { rows: n } : { error: "no count came back" };
      } catch (err) {
        count = { error: err instanceof Error ? err.message : String(err) };
      }
      if (run !== counting.current) return;
      setCounts((prev) => ({ ...prev, [e.name]: count }));
    }
  }

  useEffect(
    () => () => {
      counting.current++;
    },
    [],
  );

  useEffect(() => {
    void refresh();
    // Scheduled and agent-triggered runs land here too: each commits its
    // `collector_run` row.
    return subscribeOxplowEvents((event) => {
      if (collectorRan(event) || event.kind === "approvalsChanged") void refresh();
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
    setImpacts(Object.fromEntries(pending.map((name) => [name, "loading" as const])));
    for (const name of pending) {
      void providerDeclarationImpact(name)
        .then((e) => {
          if (live) setImpacts((prev) => ({ ...prev, [name]: e }));
        })
        .catch((e: unknown) => {
          if (live) setImpacts((prev) => ({ ...prev, [name]: { error: e instanceof Error ? e.message : String(e) } }));
        });
    }
    return () => {
      live = false;
    };
  }, [pendingKey]);

  async function run(l: CollectorListing) {
    const key = `${l.owner}/${l.spec.id}`;
    setBusy(key);
    try {
      // Approve & Run approves exactly the version this listing showed,
      // then runs it (`oxplow.collector.sync` itself never approves).
      if (!l.approved) {
        if (!l.version) throw new Error(`${key} can't be read, so it can't be approved`);
        await approveCollector(l.owner, l.spec.id, l.version);
      }
      const report = await syncCollector(l.owner, l.spec.id);
      const counts = Object.entries(report.rowCounts)
        .map(([e, n]) => `${n} ${e}`)
        .join(", ");
      showToast({ message: `Synced ${key}: ${counts || "no rows"}.` });
    } catch (e) {
      recordOpError({ label: `Run collector ${key}`, message: String(e) });
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

  if (entities === null) return <div style={mutedStyle}>Loading…</div>;
  const rows = entityRows(entities, counts);
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
              <td
                style={{ ...tdStyle, textAlign: "right", color: r.available ? undefined : "var(--text-secondary)" }}
                title={r.rowsTitle}
              >
                {r.rows}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3 style={subheadStyle}>Collectors</h3>
      {collectors.length === 0 ? (
        <div style={mutedStyle} data-testid="data-collectors-empty">
          No collectors. An extension can declare one to bring in outside data.
        </div>
      ) : (
        collectors.map((l) => {
          const s = collectorRowModel(l);
          const key = `${l.owner}/${s.id}`;
          return (
            <div key={key} data-testid={`collector-row-${key}`} style={rowStyle}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span>
                  <code>{key}</code>
                </span>
                <span style={mutedStyle}>
                  {l.spec.runtime} · {s.trigger} · {s.status}
                  {s.lastRunAt ? ` · last run ${new Date(s.lastRunAt).toLocaleString()}` : ""}
                </span>
                <span style={{ flex: 1 }} />
                <button
                  type="button"
                  data-testid={`collector-run-${key}`}
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
      <h3 style={subheadStyle} id="settings-data-programs">Programs</h3>
      {programs.length === 0 ? (
        <div style={mutedStyle} data-testid="data-programs-empty">
          This project&apos;s config runs no programs.
        </div>
      ) : (
        programs.map((p) => {
          const m = programRow(p);
          const impact = p.kind === "provider" && !p.approved ? impacts[p.name] : undefined;
          const failed = impact !== undefined && impact !== "loading" && "error" in impact ? impact.error : null;
          const diff = impact !== undefined && impact !== "loading" && !("error" in impact) ? impact : null;
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
                    disabled={busy !== null || !canApprove(p, impact)}
                    onClick={() => void approve(p)}
                  >
                    {busy === m.key ? "Approving…" : "Approve"}
                  </button>
                )}
              </div>
              {m.bundled ? <ProgramSource rowKey={m.key} program={p} /> : null}
              {p.kind === "effect" && p.approved ? <BackfillAction rowKey={m.key} effect={p.name} /> : null}
              {impact === "loading" ? (
                <div style={mutedStyle}>Comparing its declarations…</div>
              ) : failed !== null ? (
                <div data-testid={`program-impact-error-${m.key}`} style={errorStyle}>
                  Couldn&apos;t compare its declarations: {failed}
                </div>
              ) : diff ? (
                <ul data-testid={`program-impact-${m.key}`} style={{ margin: "4px 0 0", paddingLeft: 18, ...mutedStyle }}>
                  {providerImpactLines(diff).map((line, i) => (
                    <li key={i}>{line}</li>
                  ))}
                </ul>
              ) : null}
            </div>
          );
        })
      )}
      <h3 style={subheadStyle} id="settings-data-delivery">Delivery</h3>
      <DeliveryList emptyLabel="Every event reached its consumers." />
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

/** Backfill… on an approved effect's row (P9.D5): the live consumer never
 *  reacts to what was logged before the effect's approval, so a person has
 *  it react to those events — told first how many there are
 *  (`oxplow.effect.backfill_plan`), and that the effect may call outside oxplow
 *  for each. The second click is the confirmation `oxplow.effect.backfill` asks
 *  for; Escape (or Cancel) backs out. */
/** What `oxplow.effect.backfill_plan` answered: the run is bound to its range. */
interface BackfillPlan {
  planned: number;
  toSeq: number | null;
  batch: number;
}

function BackfillAction({ rowKey, effect }: { rowKey: string; effect: string }) {
  const [plan, setPlan] = useState<BackfillPlan | null>(null);
  const planned = plan?.planned ?? null;
  const [busy, setBusy] = useState(false);

  async function count() {
    setBusy(true);
    try {
      const out = await runCommand("oxplow.effect.backfill_plan", { effect });
      const r = (out.result ?? {}) as { planned?: number; to_seq?: number | null; batch?: number };
      setPlan({ planned: Number(r.planned ?? 0), toSeq: r.to_seq ?? null, batch: Number(r.batch ?? 0) });
    } catch (e) {
      recordOpError({ label: `Plan a backfill of ${effect}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  }

  // On the range it was shown (tsk849): what was logged since isn't in it.
  async function run() {
    if (plan === null || plan.toSeq === null) return;
    setBusy(true);
    try {
      const out = await runCommand("oxplow.effect.backfill", { effect, to_seq: plan.toSeq }, true);
      showToast({ message: backfillDone(out.result as BackfillResult) });
      setPlan(null);
    } catch (e) {
      recordOpError({ label: `Backfill ${effect}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  }

  if (planned === null) {
    return (
      <div style={{ marginTop: 4 }}>
        <button
          type="button"
          data-testid={`effect-backfill-${rowKey}`}
          title="Have it react to matching events logged before it was approved. You're shown how many first."
          disabled={busy}
          onClick={() => void count()}
        >
          {busy ? "Counting…" : "Backfill…"}
        </button>
      </div>
    );
  }
  return (
    <div
      data-testid={`effect-backfill-ask-${rowKey}`}
      style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}
      onKeyDown={(e) => {
        if (e.key === "Escape") setPlan(null);
      }}
    >
      <span style={mutedStyle}>{backfillAsk(effect, planned)}</span>
      <span style={{ flex: 1 }} />
      {planned > 0 ? (
        <button type="button" autoFocus data-testid={`effect-backfill-run-${rowKey}`} disabled={busy} onClick={() => void run()}>
          {busy ? "Running…" : backfillRunLabel(planned, plan?.batch ?? planned)}
        </button>
      ) : null}
      {/* With nothing to run, Close is all there is: focused, so Escape
          reaches the row's handler (tsk850). */}
      <button
        type="button"
        autoFocus={planned === 0}
        data-testid={`effect-backfill-cancel-${rowKey}`}
        disabled={busy}
        onClick={() => setPlan(null)}
      >
        {planned > 0 ? "Cancel" : "Close"}
      </button>
    </div>
  );
}

/** A bundled program's entry, shown on request: its files come with
 *  oxplow, so this is where a person reads what they approve (tsk953). */
function ProgramSource({ rowKey, program }: { rowKey: string; program: ProjectProgram }) {
  const [source, setSource] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const sourceId = `program-source-${useId()}`;
  async function toggle() {
    if (open) {
      setOpen(false);
      return;
    }
    try {
      setSource(await programSource(program.kind, program.name));
      setOpen(true);
    } catch (e) {
      recordOpError({ label: `Read ${program.name}`, message: e instanceof Error ? e.message : String(e) });
    }
  }
  return (
    <div>
      <button
        type="button"
        data-testid={`program-source-toggle-${rowKey}`}
        aria-expanded={open}
        aria-controls={sourceId}
        onClick={() => void toggle()}
      >
        {open ? "Hide the script" : "Read the script"}
      </button>
      {open && source !== null ? (
        <pre id={sourceId} data-testid={`program-source-${rowKey}`} style={{ ...mutedStyle, whiteSpace: "pre-wrap", margin: "4px 0 0" }}>
          {source}
        </pre>
      ) : null}
    </div>
  );
}
