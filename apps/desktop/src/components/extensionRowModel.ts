/** Pure row presentation for the Settings → Extensions list. */
import type { CollectorListing, EffectReport, Extension, ExtensionReview, Trigger } from "../tauri-bridge/generated/bindings.js";
import { readsChanged } from "../lens/lensRerun.js";
import { grantChanges, grantsLine, providerChanges } from "./providerEffectText.js";

export interface ExtensionRowModel {
  name: string;
  description: string;
  lensCount: number;
  /** Where it came from: "In this repo", or `<url> @ <ref> (<sha7>)`. */
  origin: string;
  /** Only git-installed extensions can be updated from their source. */
  canUpdate: boolean;
  healthy: boolean;
  errors: string[];
  enabled: boolean;
  /** "Enable" or "Disable". */
  toggleLabel: string;
  /** Shown while it's off, saying where that's set. */
  disabledNote: string | null;
}

export function extensionRowModel(ext: Extension): ExtensionRowModel {
  const src = ext.source;
  const origin =
    ext.origin === "bundled"
      ? "Ships with oxplow"
      : src
        ? `${src.git}${src.gitRef ? ` @ ${src.gitRef}` : ""} (${src.sha.slice(0, 7)})`
        : "In this repo";
  return {
    name: ext.name,
    description: ext.description,
    lensCount: ext.lenses.length,
    origin,
    canUpdate: src !== null && ext.origin !== "bundled",
    healthy: ext.errors.length === 0,
    errors: ext.errors,
    enabled: ext.enabled,
    toggleLabel: ext.enabled ? "Disable" : "Enable",
    disabledNote: ext.enabled ? null : "Off for this project (extensions.disabled in .oxplow/project.yaml).",
  };
}

export interface CollectorRowModel {
  id: string;
  /** `manual`, `every <n>m` or `on <event types>`. */
  trigger: string;
  /** "Never run", "Failed", or row counts ("12 pr · 3 review"). */
  status: string;
  lastRunAt: string | null;
  error: string | null;
  /** Unapproved collectors need a person to approve the script first. */
  action: "approve" | "sync";
  actionLabel: string;
  /** Hover text saying exactly what running it does. */
  actionTitle: string;
  /** Declared credentials and whether each has a value (never the value). */
  credentials: { name: string; set: boolean }[];
  /** "Needs X, Y (set it below)." when some are unset, else null. */
  missingCredentials: string | null;
}

/** A collector ran — by hand, on its schedule or for an event: each run
 *  commits its `collector_run` row, so `v_collector_run` changed. */
export function collectorRan(event: Readonly<Record<string, unknown>>): boolean {
  return readsChanged(event, { models: ["v_collector_run"], tables: [], measures: [] });
}

/** A collector's trigger as a short phrase. */
export function triggerLabel(t: Trigger): string {
  switch (t.kind) {
    case "manual":
      return "manual";
    case "every":
      return `every ${t.minutes}m`;
    case "on":
      return `on ${t.events.join(", ")}`;
  }
}

export function collectorRowModel(l: CollectorListing): CollectorRowModel {
  const st = l.run;
  const status = !st
    ? "Never run"
    : st.status === "error"
      ? "Failed"
      : Object.entries(st.rowCounts)
          .map(([entity, n]) => `${n} ${entity}`)
          .join(" · ") || "0 rows";
  const passed = [
    l.spec.env.length > 0 ? `${l.spec.env.join(", ")} from your environment` : null,
    l.spec.credentials.length > 0 ? `${l.spec.credentials.join(", ")} from your keychain` : null,
  ].filter(Boolean);
  const env = passed.length > 0 ? ` with ${passed.join(" and ")}` : "";
  const hosts = l.spec.network;
  const network =
    hosts.length === 0
      ? l.networkEnforced
        ? " It gets no network access."
        : " Network access isn't restricted on this OS."
      : l.networkEnforced
        ? ` It can reach only ${hosts.join(", ")}.`
        : ` It declares ${hosts.join(", ")} (not enforced on this OS).`;
  const missing = l.credentials.filter((c) => !c.set).map((c) => c.name);
  return {
    id: l.spec.id,
    trigger: triggerLabel(l.spec.trigger),
    status,
    lastRunAt: st?.lastRunAt ?? null,
    error: st?.status === "error" ? (st.error ?? "Unknown error") : null,
    action: l.approved ? "sync" : "approve",
    actionLabel: l.approved ? "Sync Now" : "Approve & Run",
    actionTitle: l.approved
      ? `Run ${l.spec.entry ?? l.spec.id} now`
      : `Runs ${l.owner}/${l.spec.entry ?? l.spec.id} on this machine${env}.${network} Approve only if you trust this extension; a changed script or host list needs approval again.`,
    credentials: l.credentials,
    missingCredentials: missing.length > 0 ? `Needs ${missing.join(", ")} (set it under Extensions).` : null,
  };
}

/// The credentials an extension's collectors declare, each once, with whether
/// it has a value (Settings → Extensions; values live in the keychain).
export function extensionCredentials(
  listings: CollectorListing[],
  extension: string,
): { name: string; set: boolean }[] {
  const out = new Map<string, boolean>();
  for (const l of listings) {
    if (l.owner !== extension) continue;
    for (const c of l.credentials) out.set(c.name, (out.get(c.name) ?? false) || c.set);
  }
  return [...out].map(([name, set]) => ({ name, set })).sort((a, b) => a.name.localeCompare(b.name));
}

/** What installing or updating an extension would bring in, as lines a
 *  person reads before confirming (tsk378). */
export interface ReviewModel {
  name: string;
  description: string;
  /** `<url> @ <ref> (<sha7>)`. */
  from: string;
  /** One line per thing it adds; programs say what they reach and read. */
  declares: string[];
  /** Load errors: it can't be installed while there are any. */
  errors: string[];
  /** What a dry run of its lenses found; shown, not blocking. */
  problems: string[];
  /** What installing it would change, one line each — grants first. */
  effects: string[];
  /** Lenses whose text changes: before and after, side by side. */
  lensDiffs: { id: string; before: string; after: string }[];
  canInstall: boolean;
}

/** The report as lines: collectors' and providers' grants first (what a
 *  person approves), then models, lenses and the config schema. */
export function effectLines(report: EffectReport): string[] {
  const out: string[] = [];
  for (const c of report.collectors) {
    if (c.change === "added") out.push(`Collector ${c.id}: added — ${c.after ? grantsLine(c.after) : "no grants"}`);
    else if (c.change === "removed") out.push(`Collector ${c.id}: removed`);
    else for (const g of grantChanges(c.before, c.after)) out.push(`Collector ${c.id}: ${g}`);
  }
  for (const p of report.providers) {
    for (const line of providerChanges(p)) out.push(`Provider ${p.id}: ${line}`);
  }
  for (const m of report.models) {
    if (m.change === "unchanged") continue;
    const what = m.change === "changed" ? (m.contractChange ?? `its ${listed(m.changed)} changed (same columns)`) : m.change;
    const downstream = m.downstream.length > 0 ? `; read by ${m.downstream.join(", ")}` : "";
    out.push(`Model ${m.view}: ${what}${downstream}`);
  }
  for (const l of report.lenses) {
    if (l.error) out.push(`Lens ${l.id}: ${l.change === "unchanged" ? "" : `${l.change}; `}its query fails: ${l.error}`);
    else if (l.change !== "unchanged") out.push(`Lens ${l.id}: ${l.change}`);
  }
  if (report.config?.otherChange) out.push(`Config: ${report.config.otherChange}`);
  if (report.config && report.config.changedKeys.length > 0) {
    out.push(`Config: ${report.config.changedKeys.join(", ")} changed`);
  }
  return out;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function reviewModel(review: ExtensionReview): ReviewModel {
  const ext = review.extension;
  const declares: string[] = [];
  if (ext.lenses.length > 0) declares.push(plural(ext.lenses.length, "lens", "lenses"));
  for (const s of ext.collectors) {
    if (s.runtime === "read") {
      declares.push(`Collector ${s.id} reads ${s.provider?.instance}'s ${s.provider?.collector}`);
    } else if (s.runtime === "exec") {
      const parts = [`Collector ${s.id} runs the program ${s.entry}`];
      parts.push(s.network.length > 0 ? `reaches ${s.network.join(", ")}` : "no network");
      if (s.credentials.length > 0) parts.push(`reads ${s.credentials.join(", ")}`);
      if (s.env.length > 0) parts.push(`reads env ${s.env.join(", ")}`);
      declares.push(`${parts.join(" · ")} (you'll approve it before it runs)`);
    } else {
      declares.push(`Collector ${s.id} runs ${s.entry} (${s.runtime}, sandboxed: no network, files or credentials)`);
    }
  }
  const advisories = ext.advisories ?? [];
  if (advisories.length > 0) {
    declares.push(
      `${plural(advisories.length, "advisory", "advisories")} shown to your agents: ${advisories.map((a) => a.id).join(", ")}`,
    );
  }
  const metrics = ext.metrics ?? [];
  if (metrics.length > 0) declares.push(plural(metrics.length, "metric"));
  if (ext.ui.slots.length > 0) {
    declares.push(`Adds to pages: ${[...new Set(ext.ui.slots.map((s) => s.slot))].join(", ")}`);
  }
  return {
    name: ext.name,
    description: ext.description,
    from: `${review.git}${review.gitRef ? ` @ ${review.gitRef}` : ""} (${review.sha.slice(0, 7)})`,
    declares,
    errors: ext.errors,
    problems: review.problems,
    // No report when the candidate doesn't load; its errors say why.
    effects: review.effects ? effectLines(review.effects) : [],
    lensDiffs: (review.effects?.lenses ?? [])
      .filter((l) => l.change === "changed" && l.before !== null && l.after !== null)
      .map((l) => ({ id: l.id, before: l.before!, after: l.after! })),
    canInstall: ext.errors.length === 0,
  };
}

/** `a`, `a and b`, `a, b and c`. */
function listed(parts: string[]): string {
  return parts.length <= 1 ? (parts[0] ?? "definition") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}
