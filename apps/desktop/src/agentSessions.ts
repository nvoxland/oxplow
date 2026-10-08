/**
 * A thread's agent sessions (`v_agent_session`, `.context/data-model.md`
 * "agent_session"): the agent slots a person opened on it. Read from the
 * model and re-read when it changes; the rows are the truth, never a copy
 * the UI keeps.
 */
import { useCallback, useEffect, useState } from "react";

import { querySql, type AgentKind } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { threadIdOf, threadRowId } from "./modelIds.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One open agent session, under the UI's ids. */
export interface AgentSessionRow {
  /** `ses3`. */
  id: string;
  /** `thr1`. */
  threadId: string;
  /** `terminal`, `chat` or `action`. */
  kind: string;
  harness: AgentKind;
  /** For an `acp` session, the ACP agent's name. */
  acpAgent: string | null;
  /** What the person called it; empty until renamed. */
  title: string;
  openedAt: string;
}

const COLUMNS = "id, thread_id, kind, harness, acp_agent, title, opened_at";

export function agentSessionsFromResult(result: SqlQueryResult): AgentSessionRow[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: `ses${Number(at(row, "id"))}`,
    threadId: threadIdOf(Number(at(row, "thread_id"))),
    kind: String(at(row, "kind")),
    harness: String(at(row, "harness")) as AgentKind,
    acpAgent: at(row, "acp_agent") == null ? null : String(at(row, "acp_agent")),
    title: String(at(row, "title") ?? ""),
    openedAt: String(at(row, "opened_at")),
  }));
}

/** A thread's open sessions, oldest first, and what was read. */
export async function readThreadSessions(threadId: string): Promise<{ sessions: AgentSessionRow[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_agent_session WHERE thread_id = ?1 AND closed_at IS NULL ORDER BY opened_at, id`,
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
