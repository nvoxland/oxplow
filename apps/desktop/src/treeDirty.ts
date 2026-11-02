/// Whether the shown stream's working tree has uncommitted changes: the
/// `treeDirty` context key a command's `when` reads (Commit Changes… is
/// offered only on a tree with something to commit). The same fact the
/// Uncommitted panel counts: the files of the stream's `working` change.

import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { streamRowId } from "./modelIds.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** The one-row answer of [`readTreeDirty`]'s query: a file is there. */
export function treeDirtyFrom(result: Pick<SqlQueryResult, "rows">): boolean {
  return Number(result.rows[0]?.[0] ?? 0) > 0;
}

export async function readTreeDirty(streamId: string): Promise<{ dirty: boolean; reads: Reads }> {
  const res = await querySql(
    `SELECT EXISTS (
       SELECT 1 FROM v_change_file f JOIN v_change c ON c.id = f.change_id
        WHERE c.stream_id = ?1 AND c.kind = 'working')`,
    [streamRowId(streamId)],
    1,
  );
  return { dirty: treeDirtyFrom(res), reads: res.reads };
}

/** Whether the stream's tree is dirty, live; false while unread or without a stream. */
export function useTreeDirty(streamId: string | null | undefined): boolean {
  const [dirty, setDirty] = useState(false);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    if (!streamId) {
      setDirty(false);
      return;
    }
    void readTreeDirty(streamId)
      .then((r) => {
        setDirty(r.dirty);
        setReads(r.reads);
      })
      .catch(() => setDirty(false));
  }, [streamId]);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return dirty;
}
