import { afterEach, expect, mock, test } from "bun:test";

// The workspace listings carry the VCS's neutral status as `status` and a
// directory's `hasChanges` straight off the wire — no hand-mapped copy of
// the generated type to drift (tsk211 was `git_status` never mapped;
// tsk520 found `listWorkspaceEntries` had the same bug).

const realBindings = await import("./tauri-bridge/generated/bindings.js");
const ok = <T,>(data: T) => ({ status: "ok" as const, data });

mock.module("./tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: {
    ...realBindings.commands,
    listWorkspaceFiles: async () =>
      ok([
        { path: "changed.ts", status: "modified" },
        { path: "clean.ts", status: null },
      ]),
    listWorkspaceEntries: async () =>
      ok([
        { name: "src", path: "src", kind: "directory", status: null, hasChanges: true },
        { name: "a.ts", path: "a.ts", kind: "file", status: "conflicted", hasChanges: true },
      ]),
    getWorkspaceStatusSummary: async () =>
      ok({ modified: 1, added: 0, deleted: 0, renamed: 0, untracked: 0, total: 1 }),
  },
}));

const api = await import("./api.js");

afterEach(() => {});

test("listWorkspaceFiles keeps each file's status (clean files stay null)", async () => {
  const { files } = await api.listWorkspaceFiles("str1");
  const byPath = Object.fromEntries(files.map((f) => [f.path, f.status]));
  expect(byPath["changed.ts"]).toBe("modified");
  expect(byPath["clean.ts"]).toBe(null);

  // The downstream "uncommitted" predicate only keeps files with a real status.
  const uncommitted = files.filter((f) => f.status !== null).map((f) => f.path);
  expect(uncommitted).toEqual(["changed.ts"]);
});

test("listWorkspaceEntries keeps status and hasChanges", async () => {
  const entries = await api.listWorkspaceEntries("str1");
  expect(entries.map((e) => [e.path, e.status, e.hasChanges])).toEqual([
    ["src", null, true],
    ["a.ts", "conflicted", true],
  ]);
});
