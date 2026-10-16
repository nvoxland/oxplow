/**
 * A thread's agent sessions (`v_agent_session`, `.context/data-model.md`
 * "agent_session"): the agent slots a person opened on it. Read from the
 * model and re-read when it changes; the rows are the truth, never a copy
 * the UI keeps.
 */
import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { threadRowId } from "./modelIds.js";
import { idOf } from "./refs/ref.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One open agent session, under the UI's ids. */
export interface AgentSessionRow {
  /** `ses3`. */
  id: string;
  /** `thr1`. */
  threadId: string;
  /** `terminal`, `chat` or `action`. */
  kind: string;
  /** The harness's key. */
  harness: string;
  /** For an `acp` session, the ACP agent's name. */
  acpAgent: string | null;
  /** What the person called it; empty until renamed. */
  title: string;
  openedAt: string;
  /** The person stopped it, and hasn't started it again: its tab spawns
   *  nothing until they do. */
  stoppedByUser: boolean;
  /** The harness's saved conversation it is bound to; empty until it has one. */
  resumeId: string;
}

const COLUMNS =
  "s.ref, (SELECT t.ref FROM v_thread t WHERE t.id = s.thread_id) AS thread_ref, s.kind, s.harness, s.acp_agent, s.title, s.opened_at, s.stopped_by_user_at, s.resume_session_id";

export function agentSessionsFromResult(result: SqlQueryResult): AgentSessionRow[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: idOf(String(at(row, "ref")), "agent_session") ?? "",
    threadId: idOf(String(at(row, "thread_ref")), "thread") ?? "",
    kind: String(at(row, "kind")),
    harness: String(at(row, "harness")),
    acpAgent: at(row, "acp_agent") == null ? null : String(at(row, "acp_agent")),
    title: String(at(row, "title") ?? ""),
    openedAt: String(at(row, "opened_at")),
    stoppedByUser: at(row, "stopped_by_user_at") != null,
    resumeId: String(at(row, "resume_session_id") ?? ""),
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

/** A thread's open sessions, re-read when the model changes; `null` until
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
