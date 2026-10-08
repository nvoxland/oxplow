import { expect, test } from "bun:test";

import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { keepBlockedReason, lineage, LINEAGE_SQL, MODEL_COLUMNS_SQL, MODELS_SQL, modelColumns, models } from "./exploreData.js";

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
        ["v_work_item", "core", "sql", "Work items."],
        ["v_gh_pr", "gh", "entity", "Pull requests."],
      ],
    ),
  );
  expect(rows).toEqual([
    { view: "v_work_item", owner: "core", kind: "sql", description: "Work items." },
    { view: "v_gh_pr", owner: "gh", kind: "entity", description: "Pull requests." },
  ]);
  expect(modelColumns(result(["name", "sql_type", "doc"], [["id", "INTEGER", "Task id."], ["n", null, "Count."]]))).toEqual([
    { name: "id", sqlType: "INTEGER", doc: "Task id." },
    { name: "n", sqlType: "", doc: "Count." },
  ]);
});

test("lineage splits what a model reads from what reads it", () => {
  const rows = result(
    ["view", "input", "kind"],
    [
      ["v_claim", "v_test_run", "ref"],
      ["v_claim", "claim", "source"],
      ["v_effort_claim", "v_claim", "ref"],
      ["v_claim_digest", "v_claim", "ref"],
    ],
  );
  expect(LINEAGE_SQL).toContain("FROM v_model_lineage WHERE view = ?1 OR input = ?1");
  expect(lineage("v_claim", rows)).toEqual({
    reads: [
      { name: "claim", kind: "source" },
      { name: "v_test_run", kind: "ref" },
    ],
    readBy: ["v_claim_digest", "v_effort_claim"],
  });
  expect(lineage("v_work_item", result(["view", "input", "kind"], []))).toEqual({ reads: [], readBy: [] });
});

test("a raw read can't be kept: saving and pinning say why", () => {
  expect(keepBlockedReason(false)).toBeNull();
  const why = keepBlockedReason(true);
  expect(why).toContain("raw tables");
  expect(why).toContain("models");
});

import { chartDefaults, metricTemplate, sliceTemplate } from "./exploreData.js";

test("the explorer's metric template: a line over captures; Slice By regenerates it with a dimension", () => {
  const plain = metricTemplate("oxplow.todos", null);
  expect(plain.sql).toContain("metric_grid('capture')");
  expect(plain.viz).toBe("line");
  expect(plain.chart).toEqual({ x: "captured_at", y: "value", series: null, label: null, size: null, group: null });
  const sliced = sliceTemplate(plain, "zone");
  expect(sliced.sql).toContain("metric_grid('capture', 'zone')");
  expect(sliced.chart.series).toBe("group");
  expect(sliceTemplate(sliced, null).sql).toBe(plain.sql);
});

test("a chart picks sensible columns from a result", () => {
  const r = result(["day", "n", "zone"], [["mon", 3, "core"]]);
  expect(chartDefaults("bar", r)).toEqual({ x: "day", y: "n", series: null, label: null, size: null, group: null });
  expect(chartDefaults("treemap", r)).toEqual({ x: null, y: null, series: null, label: "day", size: "n", group: null });
  expect(chartDefaults("table", r)).toBeNull();
});
