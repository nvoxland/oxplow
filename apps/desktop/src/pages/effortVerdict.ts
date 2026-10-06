/// What a reviewer needs first about an effort (tsk1036): did it test, how
/// much of the change the tests ran, and what's left to check — read from
/// core models only (`oxplow-bundled` is optional), as one query.
import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/** One row for effort `?1` (its row id). */
export const VERDICT_SQL = `
SELECT
  (SELECT count(*) FROM v_effort_observation WHERE effort_id = ?1 AND kind = 'test-run') AS runs,
  (SELECT json_extract(payload_json, '$.passed') FROM v_effort_observation
     WHERE effort_id = ?1 AND kind = 'test-run' ORDER BY seq DESC LIMIT 1) AS last_passed,
  (SELECT json_extract(payload_json, '$.failed') FROM v_effort_observation
     WHERE effort_id = ?1 AND kind = 'test-run' ORDER BY seq DESC LIMIT 1) AS last_failed,
  (SELECT metric_value FROM v_effort_observation
     WHERE effort_id = ?1 AND kind = 'diff-coverage' ORDER BY seq DESC LIMIT 1) AS diff_coverage,
  (SELECT count(*) FROM v_claim WHERE effort_id = ?1 AND verified = 0) AS unverified,
  (SELECT count(*) FROM v_decision WHERE effort_id = ?1 AND provenance = 'inferred') AS to_confirm,
  (SELECT count(*) FROM v_decision WHERE effort_id = ?1 AND provenance <> 'inferred') AS decisions
`;

export interface Verdict {
  runs: number;
  lastPassed: number | null;
  lastFailed: number | null;
  diffCoverage: number | null;
  unverified: number;
  toConfirm: number;
  decisions: number;
}

export interface VerdictItem {
  key: "tests" | "coverage" | "claims" | "decisions";
  text: string;
  tone: "good" | "bad" | "neutral";
}

/** Diff coverage below this needs a look. */
const COVERAGE_OK = 80;

export function verdictOf(result: SqlQueryResult): Verdict {
  const row = result.rows[0] ?? [];
  const at = (name: string): number | null => {
    const v = row[result.columns.indexOf(name)];
    return typeof v === "number" ? v : null;
  };
  return {
    runs: at("runs") ?? 0,
    lastPassed: at("last_passed"),
    lastFailed: at("last_failed"),
    diffCoverage: at("diff_coverage"),
    unverified: at("unverified") ?? 0,
    toConfirm: at("to_confirm") ?? 0,
    decisions: at("decisions") ?? 0,
  };
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function verdictItems(v: Verdict): VerdictItem[] {
  const tests: VerdictItem =
    v.runs === 0
      ? { key: "tests", text: "Tests: none ran", tone: "bad" }
      : (v.lastFailed ?? 0) > 0
        ? {
            key: "tests",
            text: `Tests: the last run failed (${v.lastFailed} of ${(v.lastPassed ?? 0) + (v.lastFailed ?? 0)})`,
            tone: "bad",
          }
        : {
            key: "tests",
            text: `${v.runs === 1 ? "Tests: the run passed" : `Tests: the last of ${v.runs} runs passed`}${
              v.lastPassed === null ? "" : ` (${v.lastPassed})`
            }`,
            tone: "good",
          };
  const coverage: VerdictItem =
    v.diffCoverage === null
      ? { key: "coverage", text: "Diff coverage: not measured", tone: "neutral" }
      : {
          key: "coverage",
          text: `Diff coverage: ${Math.round(v.diffCoverage)}%`,
          tone: v.diffCoverage >= COVERAGE_OK ? "good" : "bad",
        };
  const claims: VerdictItem =
    v.unverified === 0
      ? { key: "claims", text: "Every claim is backed", tone: "good" }
      : { key: "claims", text: plural(v.unverified, "unverified claim", "unverified claims"), tone: "bad" };
  const decisions: VerdictItem =
    v.toConfirm > 0
      ? { key: "decisions", text: plural(v.toConfirm, "decision to confirm", "decisions to confirm"), tone: "bad" }
      : { key: "decisions", text: plural(v.decisions, "decision made", "decisions made"), tone: "neutral" };
  return [tests, coverage, claims, decisions];
}
