/// "Data" section body for SettingsPage: every semantic-layer entity with
/// its provider and row count, and every extension source with its last
/// sync and a Run button (Approve & Run for an exec source nobody on this
/// machine approved yet). Credentials and enabling stay under Extensions.
/// Delivery lists the events a consumer couldn't take, with Retry and
/// Discard (confirmed inline). See `.context/semantic-layer.md`.
///
/// Usability contract (.context/usability.md): no modals; failures land in
/// opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useId, useState } from "react";

import {
  approveProjectProgram,
  programSource,
  listDataEntities,
  providerDeclarationEffects,
  listProjectPrograms,
  listCollectors,
  approveCollector,
  discardDeadLetter,
  retryDeadLetter,
  runCommand,
  syncCollector,
  subscribeOxplowEvents,
  type CollectorListing,
} from "../api.js";
import type { ProjectProgram } from "../tauri-bridge/generated/bindings.js";
import { indexRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import {
  backfillAsk, backfillRunLabel,
  backfillDone,
  canApprove,
  entityRows,
  entitySummary,
  programRow,
  providerEffectLines,
  type BackfillResult,
  type EntityRowModel,
  type ProviderEffectState,
} from "./dataSectionModel.js";
import {
  letterLine,
  reactionLine,
  useFailedReactions,
  useUndelivered,
  type FailedReaction,
  type UndeliveredEvent,
} from "../delivery.js";
import { collectorRan, collectorRowModel } from "./extensionRowModel.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function DataSection() {
  const nav = useOptionalPageNavigation();
  const [rows, setRows] = useState<EntityRowModel[] | null>(null);
  const [collectors, setCollectors] = useState<CollectorListing[]>([]);
  const [programs, setPrograms] = useState<ProjectProgram[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const undelivered = useUndelivered();
  const failedReactions = useFailedReactions();
  // What approving each unapproved provider would change, by instance.
  const [effects, setEffects] = useState<Record<string, ProviderEffectState>>({});

  const refresh = useCallback(async () => {
    try {
      const [entities, listings, progs] = await Promise.all([
        listDataEntities(),
        listCollectors(),
        listProjectPrograms(),
      ]);
      setRows(entityRows(entities));
      setCollectors(listings);
      setPrograms(progs);
    } catch (e) {
      recordOpError({ label: "List data", message: String(e) });
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Scheduled and agent-triggered runs land here too: each commits its
    // `collector_run` row.
    return subscribeOxplowEvents((event) => {
      if (collectorRan(event)) void refresh();
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

  async function run(l: CollectorListing) {
    const key = `${l.owner}/${l.spec.id}`;
    setBusy(key);
    try {
      // Approve & Run approves exactly the version this listing showed,
      // then runs it (`collector.sync` itself never approves).
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

  /** Retry or discard a dead letter; the list re-reads itself. */
  async function decide(l: UndeliveredEvent, retry: boolean) {
    setBusy(`letter-${l.id}`);
    try {
      const after = retry ? await retryDeadLetter(l.id) : await discardDeadLetter(l.id);
      if (retry) {
        showToast({ message: after.state === "retried" ? `Delivered event ${l.eventSeq} to ${l.consumer}.` : `${l.consumer} failed on it again.` });
      }
    } catch (e) {
      recordOpError({ label: `${retry ? "Retry" : "Discard"} event ${l.eventSeq} for ${l.consumer}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  /** Have an effect react again to an event it failed on: the person's
   *  second click was the confirmation `effect.retry` asks for. */
  async function retryReaction(r: FailedReaction) {
    setBusy(`reaction-${r.effect}-${r.eventId}`);
    try {
      const out = await runCommand("effect.retry", { effect: r.effect, event: `event:${r.eventId}` }, true);
      const result = out.result as { outcome?: string; reason?: string } | null;
      showToast({
        message:
          result?.outcome === "ok"
            ? `${r.effect} reacted to event ${r.eventSeq}.`
            : `${r.effect}: ${result?.outcome ?? "no outcome"}${result?.reason ? ` (${result.reason})` : ""}.`,
      });
    } catch (e) {
      recordOpError({ label: `Retry ${r.effect} on event ${r.eventSeq}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(null);
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
              {m.bundled ? <ProgramSource rowKey={m.key} program={p} /> : null}
              {p.kind === "effect" && p.approved ? <BackfillAction rowKey={m.key} effect={p.name} /> : null}
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
      <h3 style={subheadStyle}>Delivery</h3>
      {failedReactions.map((r) => {
        const key = `${r.effect}-${r.eventId}`;
        return (
          <div key={key} data-testid={`reaction-row-${key}`} style={rowStyle}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span>{reactionLine(r)}</span>
              <span style={{ flex: 1 }} />
              <InlineConfirm
                triggerLabel="Retry"
                confirmLabel="Retry"
                testIdPrefix={`reaction-retry-${key}`}
                title="Run the effect on this event again, as it is now. If the failed attempt was interrupted, a step outside oxplow may already have run — retrying sends it again."
                disabled={busy !== null}
                onConfirm={() => void retryReaction(r)}
              />
            </div>
            <div style={errorStyle}>{r.reason}</div>
          </div>
        );
      })}
      {undelivered.length === 0 ? (
        <div style={mutedStyle} data-testid="data-delivery-empty">
          Every event reached its consumers.
        </div>
      ) : (
        undelivered.map((l) => (
          <div key={l.id} data-testid={`delivery-row-${l.id}`} style={rowStyle}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span>{letterLine(l)}</span>
              <span style={{ flex: 1 }} />
              <button
                type="button"
                data-testid={`delivery-retry-${l.id}`}
                title="Run the event through its consumer again"
                disabled={busy !== null}
                onClick={() => void decide(l, true)}
              >
                {busy === `letter-${l.id}` ? "Retrying…" : "Retry"}
              </button>
              <InlineConfirm
                triggerLabel="Discard"
                confirmLabel="Discard"
                testIdPrefix={`delivery-discard-${l.id}`}
                title="Give up on this event for this consumer"
                disabled={busy !== null}
                onConfirm={() => void decide(l, false)}
              />
            </div>
            <div style={errorStyle}>{l.error}</div>
          </div>
        ))
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

/** Backfill… on an approved effect's row (P9.D5): the live consumer never
 *  reacts to what was logged before the effect's approval, so a person has
 *  it react to those events — told first how many there are
 *  (`effect.backfill_plan`), and that the effect may call outside oxplow
 *  for each. The second click is the confirmation `effect.backfill` asks
 *  for; Escape (or Cancel) backs out. */
/** What `effect.backfill_plan` answered: the run is bound to its range. */
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
      const out = await runCommand("effect.backfill_plan", { effect });
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
      const out = await runCommand("effect.backfill", { effect, to_seq: plan.toSeq }, true);
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
