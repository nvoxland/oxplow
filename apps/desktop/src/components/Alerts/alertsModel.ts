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

/** The bell: how many things need the person, red while something failed
 *  (a problem), the accent while only decisions and notices wait. */
export function alertsSummary(items: AlertItems): { count: number; tone: AlertTone } {
  const problems = items.opErrors.length + items.undelivered + items.failedReactions + items.extensionProblems.length;
  // A panel's badges are its own (its panel, and a toast that opens its
  // page); the Alerts page doesn't list them, so the bell doesn't count them.
  const count = items.proposals.length + items.hints.length + problems;
  return { count, tone: problems > 0 ? "danger" : count > 0 ? "accent" : "none" };
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
