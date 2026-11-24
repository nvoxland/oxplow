import { expect, test } from "bun:test";

import { extensionProblemLine, problemsFromResult } from "./extensionProblems.js";

const res = (rows: unknown[][]) =>
  ({ columns: ["name", "problem", "at"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as never;

test("a row per problem gathers into one problem per extension, in order, at its newest failure", () => {
  expect(problemsFromResult(res([["a", "collector x stopped after 3 failures", "2026-11-24T09:00:00Z"], ["a", "effect y is failing, 1 in a row", "2026-11-24T10:00:00Z"], ["a", "needs your approval to run", null], ["b", "bad page", null]]))).toEqual([
    { name: "a", errors: ["collector x stopped after 3 failures", "effect y is failing, 1 in a row", "needs your approval to run"], at: "2026-11-24T10:00:00Z" },
    { name: "b", errors: ["bad page"], at: null },
  ]);
});

test("the line names the first error and counts the rest", () => {
  expect(extensionProblemLine({ name: "a", errors: ["x"], at: null })).toBe("`a` isn't running as it should: x");
  expect(extensionProblemLine({ name: "a", errors: ["x", "y", "z"], at: null })).toBe("`a` isn't running as it should: x (+2 more)");
});

// PROBLEMS_SQL over stand-in views: a failing effect whose reaction can be
// retried is the Alerts "failed to react" row, not its extension's problem
// too; a disabled effect (a retry doesn't run it) stays a problem.
test("a failing effect with a retryable reaction isn't also its extension's problem", async () => {
  const { Database } = await import("bun:sqlite");
  const { PROBLEMS_SQL } = await import("./extensionProblems.js");
  const db = new Database(":memory:");
  db.exec(`
    CREATE TABLE v_contribution_health (uri TEXT, namespace TEXT, contribution TEXT, kind TEXT, state TEXT, reason TEXT, consecutive_failures INTEGER, updated_at TEXT);
    CREATE TABLE v_contribution (uri TEXT, extension TEXT);
    CREATE TABLE v_extension (name TEXT, namespace TEXT, errors TEXT, pending TEXT);
    CREATE TABLE v_effect_run (contribution TEXT, latest INTEGER, retryable INTEGER);
    INSERT INTO v_extension VALUES ('a', 'a', '[]', '[]'), ('b', 'b', '[]', '[]');
    INSERT INTO v_contribution VALUES ('e:a/fail', 'a'), ('e:a/stopped', 'a'), ('e:b/fail', 'b');
    INSERT INTO v_contribution_health VALUES
      ('e:a/fail', 'a', 'fail', 'effect', 'failing', NULL, 1, '2026-11-24T10:00:00Z'),
      ('e:a/stopped', 'a', 'stopped', 'effect', 'disabled', '3 failures in a row', 3, '2026-11-24T09:00:00Z'),
      ('e:b/fail', 'b', 'fail', 'effect', 'failing', NULL, 1, '2026-11-24T08:00:00Z');
    INSERT INTO v_effect_run VALUES ('e:a/fail', 1, 1), ('e:a/stopped', 1, 1), ('e:b/fail', 1, 0);
  `);
  expect(db.query(PROBLEMS_SQL).all()).toEqual([
    { name: "a", problem: "effect stopped stopped after 3 failures in a row", at: "2026-11-24T09:00:00Z" },
    { name: "b", problem: "effect fail is failing, 1 in a row", at: "2026-11-24T08:00:00Z" },
  ]);
});
