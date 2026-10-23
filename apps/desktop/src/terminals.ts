/// A stream's terminals are rows (`v_terminal`): shells in its worktree,
/// each named by its URI. They are opened, renamed and closed by the
/// `oxplow.terminal.*` commands (`.context/terminal.md`).

import { useCallback, useEffect, useState } from "react";

import { querySql, runCommand } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

export interface TerminalRow {
  /** `trm3`. */
  id: string;
  /** `oxplow:oxplow/terminal?id=trm3`: what the shell is keyed by. */
  uri: string;
  title: string;
}

export function terminalsFromResult(result: SqlQueryResult): TerminalRow[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: `trm${String(at(row, "id"))}`,
    uri: String(at(row, "uri")),
    title: String(at(row, "title") ?? ""),
  }));
}

/** A stream's terminals in order, and what was read. */
export async function readTerminals(streamId: string): Promise<{ terminals: TerminalRow[]; reads: Reads }> {
  const stream = Number(streamId.replace(/^str/, ""));
  const res = await querySql(
    "SELECT id, uri, title FROM v_terminal WHERE stream_id = ?1 ORDER BY position, id",
    [stream],
    1_000,
  );
  return { terminals: terminalsFromResult(res), reads: res.reads };
}

/** A stream's terminals, live; `null` until first read. */
export function useTerminals(streamId: string | null): TerminalRow[] | null {
  const [terminals, setTerminals] = useState<TerminalRow[] | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  useEffect(() => setTerminals(null), [streamId]);
  const load = useCallback(() => {
    if (!streamId) return;
    void readTerminals(streamId)
      .then((r) => {
        setTerminals(r.terminals);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read terminals", message: e instanceof Error ? e.message : String(e) });
      });
  }, [streamId]);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return terminals;
}

/** Open a terminal in `streamId`; answers its row. */
export async function openTerminal(streamId: string): Promise<TerminalRow> {
  const out = await runCommand("oxplow.terminal.open", { stream: streamId });
  const r = out.result as { id: string; uri: string; title: string };
  return { id: r.id, uri: r.uri, title: r.title };
}

export async function renameTerminal(id: string, title: string): Promise<void> {
  await runCommand("oxplow.terminal.rename", { terminal: id, title });
}

/** Close terminal `id`: its shell ends and its row goes (the person asked). */
export async function closeTerminal(id: string): Promise<void> {
  await runCommand("oxplow.terminal.close", { terminal: id }, true);
}
