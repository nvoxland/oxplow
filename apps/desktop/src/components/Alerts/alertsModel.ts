import { proposalUri } from "../../uri/uri.js";
/// What needs the person, in one place: the status bar's bell,
/// the Alerts page and the toasts all read these items. Pure.

export interface AlertItems {
  /** Agent runs waiting for the person's decision. */
  proposals: { id: string; title: string }[];
  /** Operations that failed in front of the person (the op errors store). */
  opErrors: { id: string; label: string }[];
  /** Events a consumer couldn't take. */
  undelivered: number;
  /** Effect reactions that failed. */
  failedReactions: number;
  /** Extensions with errors (held back or not running as they should),
   *  each with the line to show. */
  extensionProblems: { name: string; message: string }[];
  /** Extension panel badges that fire, each with what it opens (a URI:
   *  its badge's page). */
  badges: { id: string; title: string; message: string; open: string }[];
  /** Hints raised to the person, until they dismiss one. */
  hints: { id: number; message: string }[];
}

export type AlertTone = "none" | "accent" | "danger";

/** What a proposal's alert says, for its toast and its row alike. */
export function proposalTitle(p: { threadTitle: string | null; command: string }): string {
  return `${p.threadTitle ? `“${p.threadTitle}”` : "An agent"} wants to run ${p.command}`;
}

/** The bell: how many things need the person, red while something failed
 *  (a problem), the accent while only decisions and notices wait. */
export function alertsSummary(items: AlertItems): { count: number; tone: AlertTone } {
  const problems = items.opErrors.length + items.undelivered + items.failedReactions + items.extensionProblems.length;
  // A panel's badge that fires is a notice: the Alerts panel lists it, so the
  // icon counts it (the icon and the panel's rows say the same number).
  const count = items.proposals.length + items.hints.length + items.badges.length + problems;
  return { count, tone: problems > 0 ? "danger" : count > 0 ? "accent" : "none" };
}

/** Whether Clear has anything to put away: a hint or a failed operation.
 *  A decision is made, not cleared; an undelivered event or a failed
 *  reaction is retried or discarded on the Alerts page (discarding asks
 *  first); an extension's problem is fixed. */
export function clearable(items: AlertItems): boolean {
  return items.hints.length + items.opErrors.length > 0;
}

/** A key per item a toast announces: each proposal, op error, badge and hint,
 *  and delivery failing at all (not each failed event). */
export function alertKeys(items: AlertItems): Set<string> {
  const keys = new Set<string>();
  for (const p of items.proposals) keys.add(proposalUri(p.id));
  for (const e of items.opErrors) keys.add(`op:${e.id}`);
  if (items.undelivered > 0) keys.add("delivery");
  if (items.failedReactions > 0) keys.add("reactions");
  for (const x of items.extensionProblems) keys.add(`ext-problem:${x.name}`);
  for (const b of items.badges) keys.add(`badge:${b.id}`);
  for (const h of items.hints) keys.add(`hint:${h.id}`);
  return keys;
}

/** What to toast: the items whose key `seen` doesn't hold, in the page's
 *  order. A toast only ever offers Review — a decision is made where its
 *  preview is: `open` (a URI) when the item has its own page, else the
 *  Alerts page. */
export function newAlerts(seen: Set<string>, items: AlertItems): { key: string; message: string; open?: string }[] {
  const out: { key: string; message: string; open?: string }[] = [];
  const add = (key: string, message: string, open?: string) => {
    if (!seen.has(key)) out.push({ key, message, ...(open ? { open } : {}) });
  };
  for (const p of items.proposals) add(proposalUri(p.id), p.title);
  for (const e of items.opErrors) add(`op:${e.id}`, e.label);
  if (items.undelivered > 0) {
    add("delivery", items.undelivered === 1 ? "1 event couldn't be delivered" : `${items.undelivered} events couldn't be delivered`);
  }
  if (items.failedReactions > 0) {
    add("reactions", items.failedReactions === 1 ? "An effect failed to react" : `${items.failedReactions} effects failed to react`);
  }
  for (const x of items.extensionProblems) add(`ext-problem:${x.name}`, x.message);
  for (const b of items.badges) add(`badge:${b.id}`, `${b.title}: ${b.message}`, b.open);
  for (const h of items.hints) add(`hint:${h.id}`, h.message);
  return out;
}

// ---- the dock panel's rows ---------------------------------------------------

/** What an alert is: a decision waiting on the person, something that
 *  failed, or a notice. The panel lists them in that order. */
export type AlertKind = "decision" | "failure" | "notice";

/** What the panel's rows are built from: the sources `useAlerts` reads. */
export interface AlertSources {
  proposals: { uri: string; createdAt: string; command: string; threadTitle: string | null; preview: { summary: string; details?: string[]; destructive: boolean } }[];
  opErrors: { id: string; at: number; label: string; command: string | null; message: string | null }[];
  undelivered: { lastFailedAt: string }[];
  failedReactions: { failedAt: string | null }[];
  extensionProblems: { name: string; message: string; at: string | null }[];
  badges: { id: string; title: string; message: string; open: string }[];
  hints: { id: number; message: string; threadTitle: string | null; createdAt: string | null }[];
}

/** An ISO time as epoch ms; null when there is none. */
function epoch(iso: string | null | undefined): number | null {
  return iso ? Date.parse(iso) || null : null;
}

/** The newest of some times; null when none has one. */
function newest(times: readonly (string | null)[]): number | null {
  return times.reduce<number | null>((best, t) => {
    const at = epoch(t);
    return at != null && (best == null || at > best) ? at : best;
  }, null);
}

/** One row of the Alerts dock panel. Failed operations that say the same
 *  thing (label and message) are one row, `count` of them. */
export interface AlertRow {
  key: string;
  kind: AlertKind;
  /** Where it comes from, which says what can be done about it. */
  source: "proposal" | "op" | "delivery" | "reactions" | "extension" | "hint" | "badge";
  title: string;
  /** What it says beyond the title: an error, a summary. */
  detail: string | null;
  /** The newest of it, epoch ms; null when it has no time. */
  at: number | null;
  count: number;
  /** The command that failed or is proposed. */
  command: string | null;
  /** The page it opens: its own, else the Alerts page (none here). */
  open?: string;
  /** The failed operations it stands for, to dismiss. */
  opErrorIds?: string[];
  /** The hint it is, to dismiss. */
  hintId?: number;
  /** The proposal it is, to decide. */
  proposal?: string;
  /** The extension whose problem it is, to repair. */
  extension?: string;
}

const KIND_ORDER: Record<AlertKind, number> = { decision: 0, failure: 1, notice: 2 };

/** The panel's rows: decisions, then failures, then notices, each newest
 *  first; repeats of one failure merged. */
export function alertRows(src: AlertSources): AlertRow[] {
  const rows: AlertRow[] = [];
  for (const p of src.proposals) {
    rows.push({
      key: p.uri,
      kind: "decision",
      source: "proposal",
      title: proposalTitle(p),
      detail: [p.preview.summary, ...(p.preview.details ?? [])].filter(Boolean).join("\n") || null,
      at: epoch(p.createdAt),
      count: 1,
      command: p.command,
      open: p.uri,
      proposal: p.uri,
    });
  }
  const failed = new Map<string, AlertRow>();
  for (const e of src.opErrors) {
    const key = `op:${e.label}\u0000${e.message ?? ""}`;
    const row = failed.get(key);
    if (row) {
      row.count += 1;
      row.at = Math.max(row.at ?? 0, e.at);
      row.opErrorIds!.push(e.id);
    } else {
      failed.set(key, { key, kind: "failure", source: "op", title: e.label, detail: e.message, at: e.at, count: 1, command: e.command, opErrorIds: [e.id] });
    }
  }
  rows.push(...failed.values());
  const undelivered = src.undelivered.length;
  if (undelivered > 0) {
    rows.push({ key: "delivery", kind: "failure", source: "delivery", title: undelivered === 1 ? "1 event couldn't be delivered" : `${undelivered} events couldn't be delivered`, detail: "A consumer couldn't take them. Retry them once the cause is fixed, or discard them.", at: newest(src.undelivered.map((l) => l.lastFailedAt)), count: 1, command: null });
  }
  const reactions = src.failedReactions.length;
  if (reactions > 0) {
    rows.push({ key: "reactions", kind: "failure", source: "reactions", title: reactions === 1 ? "An effect failed to react" : `${reactions} effects failed to react`, detail: "An extension's effect failed to react to an event. Retry runs it again now.", at: newest(src.failedReactions.map((r) => r.failedAt)), count: 1, command: null });
  }
  for (const x of src.extensionProblems) {
    rows.push({ key: `ext-problem:${x.name}`, kind: "failure", source: "extension", extension: x.name, title: `${x.name} isn't running as it should`, detail: x.message, at: epoch(x.at), count: 1, command: null });
  }
  for (const h of src.hints) {
    rows.push({ key: `hint:${h.id}`, kind: "notice", source: "hint", title: h.message, detail: h.threadTitle ? `From “${h.threadTitle}”` : null, at: epoch(h.createdAt), count: 1, command: null, hintId: h.id });
  }
  for (const b of src.badges) {
    rows.push({ key: `badge:${b.id}`, kind: "notice", source: "badge", title: b.title, detail: b.message, at: null, count: 1, command: null, open: b.open });
  }
  return rows
    .map((row, i) => ({ row, i }))
    .sort((a, b) => KIND_ORDER[a.row.kind] - KIND_ORDER[b.row.kind] || (b.row.at ?? -1) - (a.row.at ?? -1) || a.i - b.i)
    .map(({ row }) => row);
}

/** How many of each kind, for the toolbar. */
export function alertCounts(rows: readonly AlertRow[]): Record<AlertKind, number> {
  const counts: Record<AlertKind, number> = { decision: 0, failure: 0, notice: 0 };
  for (const r of rows) counts[r.kind] += r.count;
  return counts;
}

/** A short age: `now`, `5m`, `3h`, `2d`, `4w`. */
export function shortAge(at: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - at) / 1000));
  if (s < 60) return "now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.round(h / 24);
  if (d < 14) return `${d}d`;
  return `${Math.round(d / 7)}w`;
}
