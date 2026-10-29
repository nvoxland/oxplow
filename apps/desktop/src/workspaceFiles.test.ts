import { expect, mock, test } from "bun:test";

// The stream's files are read from `v_file` (the local filesystem when no
// `filesystem` is named): the whole tree for the index, one folder for a
// directory check.

const realApi = await import("./api.js");
const asked: Array<{ sql: string; params: unknown[] }> = [];
let rows: Array<Array<string | number | null>> = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string, params: unknown[]) => {
    asked.push({ sql, params });
    return { rows, reads: { datasets: ["v_file"], tables: [], measures: [] } };
  },
}));
const { isWorkspaceDir, readWorkspaceIndex } = await import("./workspaceFiles.js");

test("the index reads the files of the stream from v_file", async () => {
  rows = [["src/a.rs", "modified"], ["b.txt", null]];
  const out = await readWorkspaceIndex("str3");
  expect(asked.at(-1)?.sql).toContain("FROM v_file");
  expect(asked.at(-1)?.sql).not.toContain("workspace");
  expect(asked.at(-1)?.params).toEqual([3]);
  expect(out.files).toEqual([
    { path: "src/a.rs", status: "modified" },
    { path: "b.txt", status: null },
  ]);
  expect(out.reads.datasets).toEqual(["v_file"]);
});

test("a folder check reads that folder alone, by its parent", async () => {
  rows = [[1]];
  expect(await isWorkspaceDir("str3", "src/sub/")).toBe(true);
  expect(asked.at(-1)?.sql).toContain("FROM v_file");
  expect(asked.at(-1)?.params).toEqual([3, "src", "src/sub"]);
  rows = [];
  expect(await isWorkspaceDir("str3", "nowhere")).toBe(false);
  expect(asked.at(-1)?.params).toEqual([3, "", "nowhere"]);
  expect(await isWorkspaceDir("str3", "")).toBe(true);
});
