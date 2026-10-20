/**
 * The rows of the New Session page's "Earlier sessions" list: a thread's
 * closed sessions and every harness's saved conversations, merged by the
 * backend (`oxplow_app::session_history`), which also says what taking
 * each does. This turns them into what the page draws and filters
 * (`.context/pages-and-tabs.md` "new-session").
 */
import type { EarlierAction, EarlierSession, EarlierSessions } from "../tauri-bridge/generated/bindings.js";
import { formatRelative, formatSize, formatWhen } from "./format.js";

export type EarlierStatus = "open_here" | "closed_here" | "elsewhere" | "outside" | "open_elsewhere";

export interface EarlierRow {
  /** Stable across re-reads: the session's ref, else harness + conversation. */
  key: string;
  harness: string;
  /** Its title, else the first or last thing said, else its id. */
  label: string;
  /** Where it stands, for its icon: open here, closed here, from another
   *  thread, started outside oxplow, open in another thread. */
  status: EarlierStatus;
  /** The same in words (the icon's tooltip): closed here, from another
   *  thread, outside oxplow. */
  note: string;
  /** When it was last active, short (`3 h ago`, then the date), when known. */
  when: string | null;
  /** The same exactly (`YYYY-MM-DD HH:MM`, local), for its hover. */
  whenExact: string | null;
  /** How many turns oxplow saw it run, when it ran in oxplow. */
  turns: string | null;
  /** Its saved conversation's size, when its harness says. */
  size: string | null;
  action: EarlierAction;
  /** What taking it needs: the session to reopen or show, the conversation to resume. */
  session: string | null;
  conversation: string | null;
  /** What the filter matches, lower case. */
  haystack: string;
}

function labelOf(row: EarlierSession): string {
  const c = row.conversation;
  const s = row.session;
  return (
    c?.title?.trim() ||
    s?.name?.trim() ||
    c?.first_prompt?.trim() ||
    s?.last_prompt?.trim() ||
    c?.id ||
    s?.session ||
    ""
  );
}

function statusOf(row: EarlierSession): EarlierStatus {
  const s = row.session;
  switch (row.action) {
    case "reopen":
      return "closed_here";
    case "switch":
      return "open_here";
    case "none":
      return "open_elsewhere";
    case "resume":
      if (!s) return "outside";
      return s.here ? "closed_here" : "elsewhere";
  }
}

function noteOf(row: EarlierSession): string {
  const s = row.session;
  switch (row.action) {
    case "reopen":
      return "closed here";
    case "switch":
      return "open in this thread";
    case "none":
      return s ? `open in ${s.display_name} (${s.thread_title})` : "open elsewhere";
    case "resume":
      if (!s) return "not started in oxplow";
      return s.here ? "closed here" : `from ${s.thread_title || "another thread"}`;
  }
}

/** The listing's rows, those that can't be taken (open in another thread)
 *  last, the rest in the backend's order. */
export function earlierSessionRows(listing: EarlierSessions, now: number = Date.now()): EarlierRow[] {
  const rows = listing.rows.map((row): EarlierRow => {
    const c = row.conversation;
    const s = row.session;
    const label = labelOf(row);
    const note = noteOf(row);
    const at = c?.updated_at ?? s?.closed_at ?? null;
    return {
      key: s?.session ?? `${row.harness}:${c?.id ?? ""}`,
      harness: row.harness,
      label,
      status: statusOf(row),
      note,
      when: at && formatWhen(at) ? formatRelative(at, now) : null,
      whenExact: formatWhen(at),
      turns: s && s.turns > 0 ? String(s.turns) : null,
      size: formatSize(c?.size_bytes ?? null),
      action: row.action,
      session: s?.session ?? null,
      // A native id, never a session ref: a session whose conversation the
      // harness didn't list resumes on the one it is bound to.
      conversation: c?.id ?? s?.conversation ?? null,
      haystack: [label, c?.title, c?.first_prompt, s?.last_prompt, s?.thread_title, row.harness, note]
        .filter(Boolean)
        .join("\n")
        .toLowerCase(),
    };
  });
  return [...rows.filter(takeable), ...rows.filter((r) => !takeable(r))];
}

/** The rows whose title, prompts, thread or harness contain every word of `query`. */
export function filterEarlierRows(rows: readonly EarlierRow[], query: string): EarlierRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...rows];
  return rows.filter((r) => words.every((w) => r.haystack.includes(w)));
}

/** Whether taking `row` does anything. */
export function takeable(row: Pick<EarlierRow, "action">): boolean {
  return row.action !== "none";
}
