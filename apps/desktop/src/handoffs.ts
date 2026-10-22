/// Open handoffs per thread (`v_handoff`): work another thread handed to
/// it that nobody took, declined or withdrew yet. The Navigator counts them
/// on each thread's row. See `.context/agent-model.md` "Handoffs and
/// native messaging".

import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type {
  Reads,
  SqlCell,
  SqlQueryResult,
} from "./tauri-bridge/generated/bindings.js";

export interface OpenHandoffs {
  count: number;
  /** The thread that sent the newest one, by title. */
  newestFrom: string;
}

/** By thread id (`thr3`). */
export function openHandoffsFromResult(
  result: SqlQueryResult,
): Record<string, OpenHandoffs> {
  const at = (row: SqlCell[], name: string) =>
    row[result.columns.indexOf(name)] ?? null;
  const out: Record<string, OpenHandoffs> = {};
  for (const row of result.rows) {
    out[String(at(row, "thread"))] = {
      count: Number(at(row, "n")),
      newestFrom: String(at(row, "newest_from") ?? ""),
    };
  }
  return out;
}

const OPEN_HANDOFFS = `SELECT 'thr' || h.to_thread_id AS thread, count(*) AS n,
       (SELECT ft.title FROM v_handoff n JOIN v_thread ft ON ft.id = n.from_thread_id
         WHERE n.to_thread_id = h.to_thread_id AND n.state = 'open'
         ORDER BY n.sent_at DESC, n.id DESC LIMIT 1) AS newest_from
  FROM v_handoff h
 WHERE h.state = 'open'
 GROUP BY h.to_thread_id`;

/** The open handoffs per thread, live. */
export function useOpenHandoffs(): Record<string, OpenHandoffs> {
  const [open, setOpen] = useState<Record<string, OpenHandoffs>>({});
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void querySql(OPEN_HANDOFFS, [], 1_000)
      .then((res) => {
        setOpen(openHandoffsFromResult(res));
        setReads(res.reads);
      })
      .catch((e: unknown) => {
        recordOpError({
          label: "Read handoffs",
          message: e instanceof Error ? e.message : String(e),
        });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return open;
}
