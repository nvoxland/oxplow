import { expect, test } from "bun:test";

import { withFolderChanges, type WorkspaceEntry } from "./workspaceFiles.js";

// A folder holds changes when a file anywhere under it has a status; the
// model gives a file its status and a folder none.
const dir = (path: string): WorkspaceEntry => ({ name: path.split("/").pop()!, path, kind: "directory", status: null, hasChanges: false });
const file = (path: string, status: WorkspaceEntry["status"]): WorkspaceEntry => ({ name: path.split("/").pop()!, path, kind: "file", status, hasChanges: status !== null });

test("a folder has changes when a file under it, at any depth, has a status", () => {
  const files = [
    { path: "src/a/deep/x.ts", status: "modified" as const },
    { path: "src/b.ts", status: null },
    { path: "docs/readme.md", status: null },
  ];
  const out = withFolderChanges([dir("src"), dir("docs"), dir("src/a"), file("top.ts", "added"), file("clean.ts", null)], files);
  expect(out.map((e) => [e.path, e.hasChanges])).toEqual([
    ["src", true],
    ["docs", false],
    ["src/a", true],
    ["top.ts", true],
    ["clean.ts", false],
  ]);
});
