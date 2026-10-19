import { describe, expect, test } from "bun:test";

import type { SourceCoverage, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { completeness } from "./completeness.js";

const result = (over: Partial<SqlQueryResult> = {}): SqlQueryResult => ({
  columns: ["a"],
  rows: [[1], [2]],
  truncated: false,
  reads: { models: [], tables: [], measures: [] },
  freshness: [],
  ...over,
});

// Local clock times, so the wording doesn't depend on the machine's zone.
const at = (h: number, m = 0, day = 9) => new Date(2026, 9, day, h, m).toISOString();
const now = new Date(2026, 9, 9, 15, 0);

const entry = (over: Partial<SourceCoverage> = {}): SourceCoverage => ({
  model: "v_my_gh_pull_request",
  source: "collector:my-gh/gh",
  status: "current",
  at: at(12),
  error: null,
  ...over,
});

describe("completeness", () => {
  test("a whole, current result says nothing", () => {
    expect(completeness(result({ coverage: [entry()] }), now)).toEqual({ rows: null, coverage: [] });
    expect(completeness(result(), now)).toEqual({ rows: null, coverage: [] });
  });

  test("a truncated result says how many rows it shows", () => {
    const rows = Array.from({ length: 1000 }, (_, i) => [i]);
    expect(completeness(result({ rows, truncated: true }), now).rows).toBe("Showing the first 1,000 rows.");
    expect(completeness(result({ rows: [[1]], truncated: true }), now).rows).toBe("Showing the first row.");
  });

  test("a failed source says when and why", () => {
    const c = completeness(result({ coverage: [entry({ status: "failed", error: "rate limited" })] }), now);
    expect(c.coverage).toEqual(["Pull request (collector my-gh/gh) failed at 12:00: rate limited."]);
  });

  test("each other status has its own wording", () => {
    const line = (e: Partial<SourceCoverage>) => completeness(result({ coverage: [entry(e)] }), now).coverage[0];
    expect(line({ status: "stale" })).toBe("Pull request (collector my-gh/gh) is stale; it last ran at 12:00.");
    expect(line({ status: "never", at: null })).toBe("Pull request (collector my-gh/gh) hasn't run yet.");
    expect(line({ status: "disabled", error: "failed 3 times" })).toBe(
      "Pull request (collector my-gh/gh) is disabled: failed 3 times.",
    );
  });

  test("an earlier day names the day", () => {
    const c = completeness(result({ coverage: [entry({ status: "stale", at: at(9, 5, 2) })] }), now);
    expect(c.coverage[0]).toBe("Pull request (collector my-gh/gh) is stale; it last ran on Oct 2 at 09:05.");
  });

  test("only what isn't current is listed, one line per source", () => {
    const c = completeness(
      result({
        coverage: [
          entry(),
          entry({ model: "v_my_gh_issue", source: "collector:my-gh/issues", status: "never", at: null }),
          entry({ model: "v_other_thing", source: "collector:other/thing", status: "failed", error: null }),
        ],
      }),
      now,
    );
    expect(c.coverage).toEqual([
      "Issue (collector my-gh/issues) hasn't run yet.",
      "Thing (collector other/thing) failed at 12:00.",
    ]);
  });

  test("a model that isn't named after its extension keeps its name", () => {
    const c = completeness(
      result({ coverage: [entry({ model: "v_pull_requests", status: "never", at: null })] }),
      now,
    );
    expect(c.coverage[0]).toBe("Pull requests (collector my-gh/gh) hasn't run yet.");
  });
});
