/// What a viz result doesn't say by itself, as a person reads it (pure):
/// that it shows only the first rows, and that a collector feeding a dataset
/// it read hasn't run, failed or is out of date. `.context/semantic-layer.md`.

import { idOf } from "../uri/uri.js";
import type { SourceCoverage, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

export interface Completeness {
  /** "Showing the first 1,000 rows." when the result is cut short. */
  rows: string | null;
  /** One line per source that isn't current. */
  coverage: string[];
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const two = (n: number) => String(n).padStart(2, "0");

/** `12:00` today, `Oct 2 at 09:05` another day; local time. */
function when(at: string, now: Date): { text: string; today: boolean } | null {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  const time = `${two(d.getHours())}:${two(d.getMinutes())}`;
  const today = d.toDateString() === now.toDateString();
  return { text: today ? time : `${MONTHS[d.getMonth()]} ${d.getDate()} at ${time}`, today };
}

/** A collector's URI → `my-gh/gh`. */
const sourceName = (source: string) => idOf(source, "oxplow/collector") ?? source;

/** The dataset as a noun: `v_my_gh_pull_request` fed by `my-gh/gh` is
 *  "Pull request" (the extension's prefix goes, underscores become spaces). */
function noun(dataset: string, source: string): string {
  const owner = sourceName(source).split("/")[0] ?? "";
  const prefix = `v_${owner.replace(/-/g, "_")}_`;
  const bare = dataset.startsWith(prefix) ? dataset.slice(prefix.length) : dataset.replace(/^v_/, "");
  const words = bare.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function line(c: SourceCoverage, now: Date): string | null {
  const who = `${noun(c.dataset, c.source)} (collector ${sourceName(c.source)})`;
  const ran = c.at ? when(c.at, now) : null;
  const why = c.error ? `: ${c.error}` : "";
  switch (c.status) {
    case "failed":
      return `${who} failed${ran ? ` ${ran.today ? "at" : "on"} ${ran.text}` : ""}${why}.`;
    case "stale":
      return `${who} is stale${ran ? `; it last ran ${ran.today ? "at" : "on"} ${ran.text}` : ""}.`;
    case "never":
      return `${who} hasn't run yet.`;
    case "disabled":
      return `${who} is disabled${why}.`;
    default:
      return null;
  }
}

export function completeness(result: SqlQueryResult, now: Date = new Date()): Completeness {
  const n = result.rows.length;
  const rows = result.truncated
    ? n === 1
      ? "Showing the first row."
      : `Showing the first ${n.toLocaleString("en-US")} rows.`
    : null;
  const coverage = (result.coverage ?? []).flatMap((c) => {
    const l = line(c, now);
    return l ? [l] : [];
  });
  return { rows, coverage };
}
