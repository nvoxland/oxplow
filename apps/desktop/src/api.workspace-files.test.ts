import { afterEach, expect, mock, test } from "bun:test";

// tsk211: listWorkspaceFiles must map the wire `git_status` (snake_case)
// onto our camelCase `gitStatus`. Before the fix, `f.gitStatus` was always
// undefined, so `f.gitStatus !== null` matched every file and the
// Uncommitted filter showed the whole tree.

const realBindings = await import("./tauri-bridge/generated/bindings.js");
const ok = <T,>(data: T) => ({ status: "ok" as const, data });

mock.module("./tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: {
    ...realBindings.commands,
    listWorkspaceFiles: async () =>
      ok([
        { path: "changed.ts", git_status: "modified" },
        { path: "clean.ts", git_status: null },
      ]),
    getWorkspaceStatusSummary: async () =>
      ok({ modified: 1, added: 0, deleted: 0, renamed: 0, untracked: 0, total: 1 }),
  },
}));

const api = await import("./api.js");

afterEach(() => {});

test("listWorkspaceFiles maps git_status → gitStatus (clean files stay null)", async () => {
  const { files } = await api.listWorkspaceFiles("str1");
  const byPath = Object.fromEntries(files.map((f) => [f.path, f.gitStatus]));
  expect(byPath["changed.ts"]).toBe("modified");
  expect(byPath["clean.ts"]).toBe(null);

  // The downstream "uncommitted" predicate only keeps files with a real status.
  const uncommitted = files.filter((f) => f.gitStatus !== null).map((f) => f.path);
  expect(uncommitted).toEqual(["changed.ts"]);
});
