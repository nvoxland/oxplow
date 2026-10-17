/**
 * The rows of the New Session page's "Earlier sessions" list: a thread's
 * closed sessions and every harness's saved conversations, merged by the
 * backend (`oxplow_app::session_history`), which also says what taking
 * each does. This turns them into what the page draws and filters
 * (`.context/pages-and-tabs.md` "new-session").
 */
import type { EarlierAction, EarlierSession, EarlierSessions } from "../tauri-bridge/generated/bindings.js";
import { formatSize, formatWhen } from "./conversationFormat.js";

export interface EarlierRow {
  /** Stable across re-reads: the session's ref, else harness + conversation. */
  key: string;
  harness: string;
  /** Its title, else the first or last thing said, else its id. */
  label: string;
  /** Why it stands out: closed here, from another thread, outside oxplow. */
  note: string;
  /** When it was last active (`YYYY-MM-DD HH:MM`, local), when known. */
  when: string | null;
  /** Its turns, else its size. */
  detail: string | null;
  action: EarlierAction;
  /** What taking it needs: the session to reopen or show, the conversation to resume. */
  session: string | null;
  conversation: string | null;
  /** What the filter matches, lower case. */
  haystack: string;
}

/** A session with no name of its own is named `<harness> agent_session:ses3`. */
function named(display: string, harness: string): boolean {
  return !display.startsWith(`${harness} agent_session:`);
}

function labelOf(row: EarlierSession): string {
  const c = row.conversation;
  const s = row.session;
  return (
    c?.title?.trim() ||
    (s && named(s.display_name, row.harness) ? s.display_name.trim() : "") ||
    c?.first_prompt?.trim() ||
    s?.last_prompt?.trim() ||
    c?.id ||
    s?.session ||
    ""
  );
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
    const turns = s && s.turns > 0 ? `${s.turns} turn${s.turns === 1 ? "" : "s"}` : null;
    return {
      key: s?.session ?? `${row.harness}:${c?.id ?? ""}`,
      harness: row.harness,
      label,
      note,
      when: formatWhen(c?.updated_at ?? s?.closed_at ?? null),
      detail: turns ?? formatSize(c?.size_bytes ?? null),
      action: row.action,
      session: s?.session ?? null,
      conversation: c?.id ?? s?.session ?? null,
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
