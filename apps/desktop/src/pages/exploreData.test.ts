import { expect, test } from "bun:test";

import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { MODEL_COLUMNS_SQL, MODELS_SQL, modelColumns, models } from "./exploreData.js";

const result = (columns: string[], rows: (string | number | null)[][]): SqlQueryResult => ({
  columns,
  rows,
  truncated: false,
  reads: { models: [], tables: [], measures: [] },
  freshness: [],
});

test("the catalog is the model registry, core first", () => {
  expect(MODELS_SQL).toContain("FROM v_model");
  expect(MODEL_COLUMNS_SQL).toContain("FROM v_model_column WHERE view = ?1");
  const rows = models(
    result(
      ["view", "owner", "kind", "description"],
      [
        ["v_task", "core", "sql", "Tasks."],
        ["v_gh_pr", "gh", "entity", "Pull requests."],
      ],
    ),
  );
  expect(rows).toEqual([
    { view: "v_task", owner: "core", kind: "sql", description: "Tasks." },
    { view: "v_gh_pr", owner: "gh", kind: "entity", description: "Pull requests." },
  ]);
  expect(modelColumns(result(["name", "sql_type", "doc"], [["id", "INTEGER", "Task id."], ["n", null, "Count."]]))).toEqual([
    { name: "id", sqlType: "INTEGER", doc: "Task id." },
    { name: "n", sqlType: "", doc: "Count." },
  ]);
});
