/**
 * The rows of the New Session page's "Earlier sessions" list: a thread's
 * closed sessions and every harness's saved conversations, merged by the
 * backend (`oxplow_app::session_history`), which also says what taking
 * each does. This turns them into what the page draws and filters
 * (`.context/pages-and-tabs.md` "new-session").
 */
import type { EarlierAction, EarlierSession, EarlierSessions } from "../tauri-bridge/generated/bindings.js";
import { formatSize, formatWhen } from "./format.js";

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
  /** When it was last active (`YYYY-MM-DD HH:MM`, local), when known. */
  when: string | null;
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

export function earlierSessionRows(listing: EarlierSessions): EarlierRow[] {
  return listing.rows.map((row) => {
    const c = row.conversation;
    const s = row.session;
    const label = labelOf(row);
    const note = noteOf(row);
    return {
      key: s?.session ?? `${row.harness}:${c?.id ?? ""}`,
      harness: row.harness,
      label,
      status: statusOf(row),
      note,
      when: formatWhen(c?.updated_at ?? s?.closed_at ?? null),
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
