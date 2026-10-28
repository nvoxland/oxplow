import { describe, expect, test } from "bun:test";

// Explore Data is oxplow-foundation's component; its pure half is tested
// here, where the desktop's tests run (the bundle has no test runner).
import {
  chartDefaults,
  lineage,
  LINEAGE_SQL,
  metricSeriesSql,
  metricTemplate,
  MODEL_COLUMNS_SQL,
  modelColumns,
  parseChart,
  parseTemplate,
  readView,
  sampleSql,
  sliceTemplate,
  specOf,
  templateParam,
  type Rows,
} from "../../../../extensions/oxplow-foundation/src/explore-data/model.js";

const result = (columns: string[], rows: (string | number | null)[][]): Rows => ({ columns, rows, truncated: false });

test("a model's documented columns", () => {
  expect(MODEL_COLUMNS_SQL).toContain("FROM v_field WHERE view = ?1");
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

test("a metric template and a chart ride in the address as text and read back", () => {
  const plain = metricTemplate("oxplow.todos", null);
  expect(templateParam(plain)).toBe("oxplow.todos");
  expect(parseTemplate("oxplow.todos")).toEqual(plain);
  const sliced = metricTemplate("oxplow.todos", "oxplow.language");
  expect(templateParam(sliced)).toBe("oxplow.todos,oxplow.language");
  expect(parseTemplate("oxplow.todos,oxplow.language")).toEqual(sliced);
  expect(parseTemplate("")).toBeNull();

  expect(parseChart(JSON.stringify({ x: "t", y: "v" }))).toEqual({ x: "t", y: "v" });
  expect(parseChart("")).toBeNull();
  expect(parseChart("{not json")).toBeNull();
  expect(parseChart("[1]")).toBeNull();
});

test("the view reads back from the tab's address, a chart viz only by name", () => {
  expect(readView({})).toEqual({ model: "", sql: "", viz: "table", chart: null, template: null });
  const v = readView({ model: "v_effort", sql: "SELECT 1", viz: "bar", chart: JSON.stringify({ x: "a", y: "b" }) });
  expect(v).toMatchObject({ model: "v_effort", sql: "SELECT 1", viz: "bar", chart: { x: "a", y: "b" } });
  expect(readView({ viz: "pie" }).viz).toBe("table");
  expect(sampleSql("v_work_item")).toBe("SELECT * FROM v_work_item LIMIT 50");
});

// What the host draws under the explorer (`showView`) is an inline page:
// its query as a `sql` loader, its viz (and chart) over it. JSON is YAML.
test("the view shown is an inline page of the query's viz over one sql loader", () => {
  expect(JSON.parse(specOf("SELECT 1", "table", null))).toEqual({
    title: "Explore",
    data: { rows: { sql: "SELECT 1" } },
    view: { id: "rows", viz: "table", from: "rows" },
  });
  const chart = { x: "day", y: "n", series: null };
  expect(JSON.parse(specOf("SELECT day, n FROM t", "line", chart)).view).toEqual({ id: "rows", viz: "line", from: "rows", chart });
});

describe("metrics read through SQL", () => {
  test("a metric's captures, grouped and bounded", () => {
    expect(metricSeriesSql("oxplow.coverage.abs_pct")).toBe(
      [
        `SELECT g.capture_id, g.bucket AS captured_at, MEASURE('oxplow.coverage.abs_pct') AS value, NULL AS "group",`,
        "       c.branch, c.provenance, c.closest_vcs_rev AS vcs_rev, c.source",
        "FROM metric_grid('capture') g LEFT JOIN v_capture c ON c.id = g.capture_id",
        "WHERE MEASURE('oxplow.coverage.abs_pct') IS NOT NULL",
        "ORDER BY g.bucket DESC",
      ].join("\n"),
    );
    const grouped = metricSeriesSql("it's", "zone", true);
    expect(grouped).toContain("MEASURE('it''s')");
    expect(grouped).toContain(`metric_grid('capture', 'zone')`);
    expect(grouped).toContain(`g."zone" AS "group"`);
    expect(grouped).toContain("AND g.bucket >= ?1 AND g.bucket <= ?2");
  });

});
