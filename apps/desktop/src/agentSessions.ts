/**
 * A thread's agent sessions (`v_agent_session`, `.context/data-model.md`
 * "agent_session"): the agent slots a person opened on it. Read from the
 * dataset and re-read when it changes; the rows are the truth, never a copy
 * the UI keeps.
 */
import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import { threadRowId } from "./modelIds.js";
import { idOf } from "./uri/uri.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One open agent session, under the UI's ids. */
export interface AgentSessionRow {
  /** `ses3`. */
  id: string;
  /** `thr1`. */
  threadId: string;
  /** `terminal` or `action`. */
  kind: string;
  /** The harness's key. */
  harness: string;
  /** What the person called it (oxplow's alias); empty until renamed. */
  title: string;
  /** The name its harness gives the conversation; empty until it has one. */
  nativeTitle: string;
  /** What the tab says: the view's one rule (`v_agent_session.display_name`) —
   *  the alias, else the native name, else the harness and the day. */
  displayName: string;
  openedAt: string;
  /** The person stopped it, and hasn't started it again: its tab spawns
   *  nothing until they do. */
  stoppedByUser: boolean;
}

const COLUMNS =
  "s.uri, (SELECT t.uri FROM v_thread t WHERE t.id = s.thread_id) AS thread_uri, s.kind, s.harness, s.title, s.native_title, s.display_name, s.opened_at, s.stopped_by_user_at";

export function agentSessionsFromResult(result: SqlQueryResult): AgentSessionRow[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: idOf(String(at(row, "uri")), "oxplow/agent_session") ?? "",
    threadId: idOf(String(at(row, "thread_uri")), "oxplow/thread") ?? "",
    kind: String(at(row, "kind")),
    harness: String(at(row, "harness")),
    title: String(at(row, "title") ?? ""),
    nativeTitle: String(at(row, "native_title") ?? ""),
    displayName: String(at(row, "display_name") ?? ""),
    openedAt: String(at(row, "opened_at")),
    stoppedByUser: at(row, "stopped_by_user_at") != null,
  }));
}

/** A thread's open sessions, oldest first, and what was read. */
export async function readThreadSessions(threadId: string): Promise<{ sessions: AgentSessionRow[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_agent_session s WHERE s.thread_id = ?1 AND s.closed_at IS NULL ORDER BY s.opened_at, s.id`,
    [threadRowId(threadId)],
    1_000,
  );
  return { sessions: agentSessionsFromResult(res), reads: res.reads };
}

/** A thread's open sessions, re-read when the dataset changes; `null` until
 *  the first read lands. */
export function useThreadSessions(threadId: string | null): AgentSessionRow[] | null {
  const [sessions, setSessions] = useState<AgentSessionRow[] | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    if (!threadId) return;
    void readThreadSessions(threadId)
      .then((r) => {
        setSessions(r.sessions);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read this thread's agent sessions", message: e instanceof Error ? e.message : String(e) });
      });
  }, [threadId]);
  useEffect(() => {
    setSessions(null);
    load();
  }, [load]);
  useRerunOnChange(reads, load);
  return sessions;
}
