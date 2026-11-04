import { afterAll, expect, mock, test } from "bun:test";

const realApi = { ...(await import("./api.js")) }; // a copy: `mock.module` patches a loaded module in place
const asked: string[] = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string) => {
    asked.push(sql);
    return {
      columns: ["key", "value", "origin", "extension", "doc", "schema"],
      rows: [
        ["snapshotRetentionDays", "3", "project", null, "How long snapshots are kept.", '{"type":"integer"}'],
        ["ai.roles.main", "null", "default", null, "The main role.", null],
      ],
      truncated: false,
      reads: { datasets: ["v_config_value"], tables: [], measures: [] },
      freshness: [],
    };
  },
}));

const { readSettings } = await import("./configValues.js");

test("every setting is read from v_config_value, its value and schema as JSON", async () => {
  const { settings, reads } = await readSettings();
  expect(asked[0]).toContain("FROM v_config_value");
  expect(settings).toEqual([
    {
      key: "snapshotRetentionDays",
      value: 3,
      origin: "project",
      extension: null,
      doc: "How long snapshots are kept.",
      schema: { type: "integer" },
    },
    { key: "ai.roles.main", value: null, origin: "default", extension: null, doc: "The main role.", schema: null },
  ]);
  expect(reads.datasets).toEqual(["v_config_value"]);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("./api.js", () => realApi);
});
