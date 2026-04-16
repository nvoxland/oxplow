import { describe, expect, test } from "bun:test";
import { relativePathFromFileUri, streamFileUri, toEditorNavigationTarget } from "./lsp.js";

const stream = {
  id: "s-1",
  title: "proj1",
  summary: "",
  branch: "main",
  branch_ref: "refs/heads/main",
  branch_source: "local" as const,
  worktree_path: "/tmp/proj1",
  created_at: "2024-01-01T00:00:00.000Z",
  updated_at: "2024-01-01T00:00:00.000Z",
  panes: { working: "a:1.0", talking: "a:1.1" },
  resume: { working_session_id: "", talking_session_id: "" },
};

describe("streamFileUri helpers", () => {
  test("builds a real file URI inside the stream worktree", () => {
    expect(streamFileUri(stream, "src/App.tsx")).toBe("file:///tmp/proj1/src/App.tsx");
  });

  test("maps file URIs back to stream-relative paths", () => {
    expect(relativePathFromFileUri(stream, "file:///tmp/proj1/src/App.tsx")).toBe("src/App.tsx");
    expect(relativePathFromFileUri(stream, "file:///tmp/other/App.tsx")).toBeNull();
  });

  test("creates editor navigation targets from LSP locations", () => {
    expect(
      toEditorNavigationTarget(stream, "file:///tmp/proj1/src/App.tsx", {
        start: { line: 4, character: 2 },
      }),
    ).toEqual({
      path: "src/App.tsx",
      line: 5,
      column: 3,
    });
  });
});
