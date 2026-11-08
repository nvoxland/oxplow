/**
 * Extensions that have problems, as the Extension problems page lists
 * them (the one meaning of "a problem", `.context/extensions-permissions.md`):
 * a contribution that failed or stopped after failing
 * (`v_contribution_health`), permissions waiting for approval
 * (`v_extension.pending`) and load errors (`v_extension.errors`). An
 * extension with a problem counts whether or not it is enabled — a held
 * one is listed with the reasons it is held, even `oxplow-foundation`.
 * The bell counts each extension once; the Alerts page shows the same
 * page, embedded.
 */
import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One extension with errors. */
export interface ExtensionProblem {
  name: string;
  errors: string[];
}

/** One row per problem (`name`, `problem`), in the page's order, gathered
 *  per extension in the order each first appears. */
export function problemsFromResult(result: SqlQueryResult): ExtensionProblem[] {
  const name = result.columns.indexOf("name");
  const problem = result.columns.indexOf("problem");
  const out = new Map<string, ExtensionProblem>();
  for (const row of result.rows) {
    const n = String(row[name]);
    const p = out.get(n) ?? { name: n, errors: [] };
    p.errors.push(String(row[problem] ?? ""));
    out.set(n, p);
  }
  return [...out.values()];
}

/** The problems, one row each, as the Extension problems page reads them.
 *  Health is the namespace's: a row is listed under the extension that
 *  declares the contribution now. */
export const PROBLEMS_SQL = `SELECT name, problem FROM (
  SELECT COALESCE((SELECT c.extension FROM v_contribution c WHERE c.uri = h.uri),
                  (SELECT x.name FROM v_extension x WHERE x.namespace = h.namespace ORDER BY x.name LIMIT 1)) AS name,
         1 AS rank, h.kind || ' ' || h.contribution || ' ' ||
         CASE h.state WHEN 'disabled' THEN 'stopped after ' || coalesce(h.reason, 'repeated failures')
                      ELSE 'is failing, ' || h.consecutive_failures || ' in a row' END AS problem
  FROM v_contribution_health h WHERE h.state <> 'ok'
  UNION ALL
  SELECT e.name, 2, json_extract(j.value, '$.shown') FROM v_extension e, json_each(e.errors) j
  UNION ALL
  SELECT e.name, 3, 'needs your approval to run' FROM v_extension e WHERE json_array_length(e.pending) > 0
) ORDER BY name, rank`;

/** What an Alerts row says. */
export function extensionProblemLine(p: ExtensionProblem): string {
  const more = p.errors.length > 1 ? ` (+${p.errors.length - 1} more)` : "";
  return `\`${p.name}\` isn't running as it should: ${p.errors[0] ?? "unknown error"}${more}`;
}

/** The extensions with errors, and what was read. */
export async function readExtensionProblems(): Promise<{ problems: ExtensionProblem[]; reads: Reads }> {
  const res = await querySql(PROBLEMS_SQL, [], 200);
  return { problems: problemsFromResult(res), reads: res.reads };
}

/** The extensions with errors, re-read when they change. */
export function useExtensionProblems(): ExtensionProblem[] {
  const [problems, setProblems] = useState<ExtensionProblem[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readExtensionProblems()
      .then((r) => {
        setProblems(r.problems);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read extension problems", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return problems;
}
