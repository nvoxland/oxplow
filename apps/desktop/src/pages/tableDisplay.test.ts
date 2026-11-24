import { expect, test } from "bun:test";
import { parse } from "yaml";
import { EMPTY_RESULT } from "../viz/nodeRun.js";
import { displayTable, resolveTableDisplay, tablePageYaml } from "./tableDisplay.js";
import { fakeViz, tablePage } from "./testing/fakeRun.js";

test("defaults a line to a date and a number, and keeps chosen fields while they still exist", () => {
  const result = { ...EMPTY_RESULT, columns: ["label", "day", "total"], rows: [["A", "2026-01-01", 8]] };
  expect(resolveTableDisplay({ viz: "line", fields: { y: "removed", series: "label" } }, result))
    .toEqual({ viz: "line", fields: { x: "day", y: "total", series: "label" } });
});

test("a number from a temporary table saves the chosen column through the source loader", () => {
  const field = 'total "net"';
  const result = { ...EMPTY_RESULT, columns: ["label", field], rows: [["A", 8]] };
  const display = resolveTableDisplay({ viz: "number", fields: {} }, result);
  const preview = displayTable({ config: fakeViz(), params: {}, result }, display);
  expect(preview.result.rows[0]).toEqual([8, "A"]);

  const run = { ...tablePage(), params: { scope: "open" } };
  const temp = `title: Results
params: [{ name: scope }]
loaders:
  items: { sql: 'SELECT :scope AS label, 8 AS "total ""net"""' }
view: { viz: table, from: items }
`;
  expect(parse(tablePageYaml(run, "items", display, "Total", temp))).toEqual({
    title: "Total",
    params: [{ name: "scope", default: "open" }],
    loaders: {
      items: { sql: 'SELECT :scope AS label, 8 AS "total ""net"""' },
      number_value: { sql: 'SELECT source."total ""net""" FROM "items" AS source' },
    },
    view: { viz: "number", from: "number_value" },
  });
});
