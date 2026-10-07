/// Hints raised to the person (`v_agent_nudge` rows with audience
/// `person`): an extension's advisory meant for them, or oxplow muting a
/// hint that kept firing at the agent. Alerts lists them until the person
/// dismisses one (`oxplow.hint.dismiss`). See `.context/extensions.md`
/// "Advisories".

import { useCallback, useEffect, useState } from "react";

import { querySql, runCommand } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

export interface PersonHint {
  /** The nudge row (`v_agent_nudge.id`). */
  id: number;
  /** `<extension>/<id>`, or `hint-muted`. */
  kind: string;
  message: string;
  /** The thread it was raised on, by title. */
  threadTitle: string | null;
}

export function hintsFromResult(result: SqlQueryResult): PersonHint[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: Number(at(row, "id")),
    kind: String(at(row, "kind")),
    message: String(at(row, "message") ?? ""),
    threadTitle: at(row, "thread_title") == null ? null : String(at(row, "thread_title")),
  }));
}

/** The hints waiting for the person, newest first, and what was read. */
export async function readPersonHints(): Promise<{ hints: PersonHint[]; reads: Reads }> {
  const res = await querySql(
    `SELECT n.id, n.kind, n.message, (SELECT title FROM v_thread th WHERE th.id = n.thread_id) AS thread_title
       FROM v_agent_nudge n
      WHERE n.audience = 'person' AND n.delivered_at IS NULL
      ORDER BY n.id DESC`,
    [],
    1_000,
  );
  return { hints: hintsFromResult(res), reads: res.reads };
}

/** The hints waiting for the person, live. */
export function usePersonHints(): PersonHint[] {
  const [hints, setHints] = useState<PersonHint[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readPersonHints()
      .then((r) => {
        setHints(r.hints);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read hints", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return hints;
}

/** The person dismisses a hint raised to them. */
export async function dismissHint(id: number): Promise<void> {
  await runCommand("oxplow.hint.dismiss", { nudge: id });
}
