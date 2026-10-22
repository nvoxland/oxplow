/**
 * Extensions that have problems: read from `v_extension` (`errors`, a JSON
 * array of strings). An extension with errors counts whether or not it is
 * enabled — a held one is listed with the reasons it is held, even
 * `oxplow-foundation`. Alerts lists each, with a way to Settings →
 * Extensions.
 */
import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One extension with errors. */
export interface ExtensionProblem {
  name: string;
  errors: string[];
}

function parseErrors(raw: unknown): string[] {
  if (Array.isArray(raw)) return raw.map(String);
  if (typeof raw !== "string" || raw === "") return [];
  try {
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) ? v.map(String) : [raw];
  } catch {
    return [raw];
  }
}

export function problemsFromResult(result: SqlQueryResult): ExtensionProblem[] {
  return result.rows.map((row) => ({
    name: String(row[result.columns.indexOf("name")]),
    errors: parseErrors(row[result.columns.indexOf("errors")]),
  }));
}

/** What an Alerts row says. */
export function extensionProblemLine(p: ExtensionProblem): string {
  const more = p.errors.length > 1 ? ` (+${p.errors.length - 1} more)` : "";
  return `\`${p.name}\` isn't running as it should: ${p.errors[0] ?? "unknown error"}${more}`;
}

/** The extensions with errors, and what was read. */
export async function readExtensionProblems(): Promise<{ problems: ExtensionProblem[]; reads: Reads }> {
  const res = await querySql(
    `SELECT name, enabled, errors, deployment FROM v_extension
      WHERE json_array_length(errors) > 0 ORDER BY name`,
    [],
    200,
  );
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
