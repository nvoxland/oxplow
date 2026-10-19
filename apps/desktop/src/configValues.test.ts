import { expect, mock, test } from "bun:test";

const realApi = await import("./api.js");
const asked: string[] = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string) => {
    asked.push(sql);
    return {
      columns: ["key", "value", "origin", "extension", "human_only", "doc", "schema"],
      rows: [
        ["snapshotRetentionDays", "3", "project", null, 0, "How long snapshots are kept.", '{"type":"integer"}'],
        ["ai.roles.main", "null", "default", null, 1, "The main role.", null],
      ],
      truncated: false,
      reads: { models: ["v_config_value"], tables: [], measures: [] },
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
      humanOnly: false,
      doc: "How long snapshots are kept.",
      schema: { type: "integer" },
    },
    { key: "ai.roles.main", value: null, origin: "default", extension: null, humanOnly: true, doc: "The main role.", schema: null },
  ]);
  expect(reads.models).toEqual(["v_config_value"]);
});
