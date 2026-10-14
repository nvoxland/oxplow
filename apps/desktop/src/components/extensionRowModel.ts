/** Pure row presentation for the Settings → Extensions list. */
import { contributionOf } from "../refs/ref.js";
import type { CollectorListing, Extension, SecretRef, ExtensionReview, ScriptServices, ServiceListing, Trigger } from "../tauri-bridge/generated/bindings.js";
import { readsChanged } from "../lens/lensRerun.js";

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

/** `ext`'s row; `checked` is what its check (`validate_extension`, the dry
 *  run `oxplow extension check` prints) found — a lens whose SQL fails passes
 *  loading, so its errors show only there (tsk1039). Each error once. */
export function extensionRowModel(ext: Extension, checked: string[] = []): ExtensionRowModel {
  const errors = [...new Set([...ext.errors, ...checked])];
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
    healthy: errors.length === 0,
    errors,
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
  /** Unapproved collectors need a person to approve the script first; the
   *  project's own are approved under Programs (they have their own row
   *  there), not by this button. */
  action: "approve" | "approve-in-programs" | "sync";
  actionLabel: string;
  /** Hover text saying exactly what running it does. */
  actionTitle: string;
  /** Declared credentials and whether each has a value (never the value). */
  credentials: { name: string; set: boolean }[];
  /** "Needs X, Y (set it below)." when some are unset, else null. */
  missingCredentials: string | null;
  /** Why failures turned it off on this machine, else null. */
  disabled: string | null;
  /** A disabled collector can be turned back on. */
  canEnableAgain: boolean;
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
    case "onRun":
      return t.run === "test" ? "after a test run" : "after an analysis run";
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
          .join(" · ") ||
        // A fact collector records facts, not rows (tsk1044).
        (l.spec.facts.length > 0 ? "Recorded facts" : "0 rows");
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
  const program = `${l.owner}/${l.spec.entry ?? l.spec.id}`;
  // A derived collector asks only when its script calls a model.
  const consent =
    l.spec.runtime === "starlark"
      ? `${program} calls a model through your AI provider, with its key, each time it runs (${triggerLabel(l.spec.trigger)}).`
      : `Runs ${program} on this machine${env}.${network}`;
  return {
    id: l.spec.id,
    trigger: triggerLabel(l.spec.trigger),
    status,
    lastRunAt: st?.lastRunAt ?? null,
    error: st?.status === "error" ? (st.error ?? "Unknown error") : null,
    action: l.approved ? "sync" : l.owner === "project" ? "approve-in-programs" : "approve",
    actionLabel: l.approved ? "Sync Now" : l.owner === "project" ? "Approve under Programs" : "Approve & Run",
    actionTitle: l.approved
      ? `Run ${l.spec.entry ?? l.spec.id} now`
      : l.owner === "project"
        ? `${l.spec.entry ?? l.spec.id} runs a program from this project's config. Approve it in its row under Programs below, then sync it here.`
        : `${consent} Approve only if you trust this extension; a changed script or host list needs approval again.`,
    credentials: l.credentials,
    missingCredentials: missing.length > 0 ? `Needs ${missing.join(", ")} (set it under Extensions).` : null,
    disabled: l.disabled ? `Off on this machine: ${l.disabled}` : null,
    canEnableAgain: !!l.disabled,
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

/// A credential an extension's provider declares with a host secret
/// reference: the settings form sets, rotates and revokes its value (it
/// goes to the keychain and never comes back).
export interface SecretCredential {
  provider: string;
  name: string;
  reference: SecretRef;
}

/// The credentials of an extension's providers that carry a `secretRef`.
export function secretCredentials(extension: Extension): SecretCredential[] {
  const out: SecretCredential[] = [];
  for (const p of extension.providers ?? []) {
    for (const c of p.credentials) {
      if (c.secretRef) out.push({ provider: p.id, name: c.name, reference: c.secretRef });
    }
  }
  return out;
}

/// What a secret row shows: `isSet` is null until the host has answered.
export function secretRowModel(isSet: boolean | null): {
  status: string;
  setLabel: string;
  canRevoke: boolean;
} {
  if (isSet === null) return { status: "Checking…", setLabel: "Set", canRevoke: false };
  return isSet
    ? { status: "Set", setLabel: "Rotate", canRevoke: true }
    : { status: "Not set", setLabel: "Set", canRevoke: false };
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
  canInstall: boolean;
  /** What each of its scripts is given (`serviceLines`). */
  services: ServiceLine[];
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** `event_type` → `Event type`: how a person names a kind of contribution. */
const kindTitle = (kind: string) => {
  const words = kind.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
};

export function reviewModel(review: ExtensionReview, listing: readonly ServiceListing[] = []): ReviewModel {
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
  // Every other contribution, one row each: the kinds above say more.
  for (const d of ext.contributions ?? []) {
    if (d.id.kind === "lens" || d.id.kind === "collector") continue;
    declares.push(`${kindTitle(d.id.kind)} ${d.id.local}`);
  }
  return {
    name: ext.name,
    description: ext.description,
    from: `${review.git}${review.gitRef ? ` @ ${review.gitRef}` : ""} (${review.sha.slice(0, 7)})`,
    declares,
    errors: ext.errors,
    problems: review.problems,
    canInstall: ext.errors.length === 0,
    services: serviceLines(review.services, listing),
  };
}

/** One script and the services it is given, in a person's words. */
export interface ServiceLine {
  /** "Collector paths", "Effect stamp". */
  script: string;
  /** The services' titles, " · "-separated: "Read the captured tree · Ask a
   *  model (you approve it)". */
  text: string;
}

/** Each script's services as lines (Settings → Extensions, beside the
 *  grants): the service titles (`list_services`), a model call marked as
 *  yours to approve, and a service the script uses without naming it in
 *  its `needs` flagged. A script with no services has no line. */
export function serviceLines(scripts: readonly ScriptServices[], listing: readonly ServiceListing[]): ServiceLine[] {
  const title = (scope: string) => listing.find((s) => s.scope === scope)?.title ?? scope;
  const out: ServiceLine[] = [];
  for (const s of scripts) {
    if (s.services.length === 0) continue;
    const words = s.script.replace(/`/g, "");
    out.push({
      script: words.charAt(0).toUpperCase() + words.slice(1),
      text: s.services
        .map((u) => `${title(u.scope)}${u.approval ? " (you approve it)" : ""}${u.declared === "undeclared" ? " (not in its needs)" : ""}`)
        .join(" · "),
    });
  }
  return out;
}

/** The latest run of each contribution — the rows Settings → Extensions
 *  reads (`v_extension_run`, `latest = 1`). */
export const LATEST_RUNS_SQL =
  "SELECT extension, contribution, outcome, error, finished_at, decisions FROM v_extension_run WHERE latest = 1";

export interface RunRow {
  extension: string;
  /** its ref (`<kind>:<extension>/<id>`, `command:<bus id>`). */
  contribution: string;
  outcome: string;
  error: string | null;
  finished_at: string;
  /** JSON: what its grant decided, `[{ need, name?, decision, reason? }]`. */
  decisions: string | null;
}

/** An extension's last run as its row shows it. */
export interface LastRun {
  text: string;
  /** Why, for a run that failed, was denied or went stale. */
  reason: string | null;
  /** Failed, denied and stale are shown in the warning color. */
  warn: boolean;
}

const localWhen = (iso: string) => new Date(iso).toLocaleString();

/** The decision that refused a run, as a person reads it: `host evil.example:
 *  it isn't in the grant`. */
function deniedReason(decisions: string | null): string | null {
  if (!decisions) return null;
  try {
    const list = JSON.parse(decisions) as Array<{ need?: string; name?: string; decision?: string; reason?: string }>;
    const d = list.find((x) => x.decision === "denied");
    if (!d) return null;
    const what = [d.need, d.name].filter(Boolean).join(" ");
    return d.reason ? (what ? `${what}: ${d.reason}` : d.reason) : what || null;
  } catch {
    return null;
  }
}

/** How bad a warning is: a refusal or a change under the run says more
 *  than a failure. `0` for a run that is no warning. */
const SEVERITY: Record<string, number> = { denied: 3, stale: 2, failed: 1 };

/** Each extension's run to show, from the latest run of each of its
 *  contributions: the worst current warning (denied, then stale, then
 *  failed; the newest of equals), so a failing contribution isn't hidden
 *  behind another's later ok run — else the most recent run. */
export function lastRuns(rows: readonly RunRow[], when: (iso: string) => string = localWhen): Record<string, LastRun> {
  const shown = new Map<string, RunRow>();
  const outranks = (a: RunRow, b: RunRow) => {
    const [sa, sb] = [SEVERITY[a.outcome] ?? 0, SEVERITY[b.outcome] ?? 0];
    return sa !== sb ? sa > sb : a.finished_at > b.finished_at;
  };
  for (const r of rows) {
    const seen = shown.get(r.extension);
    if (!seen || outranks(r, seen)) shown.set(r.extension, r);
  }
  return Object.fromEntries(
    [...shown].map(([extension, r]) => {
      const named = contributionOf(r.contribution);
      const [kind, local] = named ? [named.kind, named.id] : ["", r.contribution];
      const warn = (SEVERITY[r.outcome] ?? 0) > 0;
      const reason = !warn ? null : r.outcome === "denied" ? (deniedReason(r.decisions) ?? r.error) : r.error;
      return [
        extension,
        { text: `Last run: ${r.outcome} · ${kind} ${local} · ${when(r.finished_at)}`, reason, warn },
      ];
    }),
  );
}
