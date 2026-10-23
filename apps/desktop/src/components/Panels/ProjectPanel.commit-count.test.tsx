import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// tsk145: the Files page "Commit (N)" button must reflect the changeset
// (the same "N changed" the header shows), not the total file count. Both
// are now driven by the workspace status's changed-path count. This test
// mounts ProjectPanel with a status summary of 2 changed files while the
// file index lists more entries, and asserts the button reads "Commit (2)"
// — i.e. it tracks the changed count, not the file count.
//
// We override only the workspace read commands on the bindings module and
// `listen` on the transport (spreading the reals so sibling tests keep the
// full surface), then dynamically import the component.

const realTransport = await import("../../tauri-bridge/transport.js");
const realBindings = await import("../../tauri-bridge/generated/bindings.js");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

// Five indexed files — more than the 2 changed — so a button bound to the
// file count would read "Commit (5)" and fail the assertion below.
const FILES = [
  { path: "a.ts", status: "modified" },
  { path: "b.ts", status: "untracked" },
  { path: "c.ts", status: null },
  { path: "d.ts", status: null },
  { path: "e.ts", status: null },
];

mock.module("../../tauri-bridge/transport.js", () => ({
  ...realTransport,
  listen: async () => () => {},
}));

mock.module("../../tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: {
    ...realBindings.commands,
    gitChangeScopes: async () =>
      ok({
        current_branch: "main",
        branch_base: null,
        upstream: null,
        on_default_branch: true,
        staged: [],
        unstaged: [],
      }),
  },
}));

// The status counts are a read of the working change (`streamStatusCounts`);
// other files mock `api.js` process-wide, so this one names the listing it
// wants: two changed files — one modified, one untracked.
const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  // The files are the model's rows (`v_workspace_file`): the whole tree's
  // walk, and no folder has entries.
  querySql: async (sql: string) => ({
    columns: [],
    rows: sql.includes("kind = 'file'") ? FILES.map((f) => [f.path, f.status]) : [],
    truncated: false,
    reads: { models: ["v_workspace_file"], tables: [], measures: [] },
    freshness: {},
  }),
  streamStatusCounts: async () => ({
    counts: { added: 0, modified: 1, deleted: 0, renamed: 0, untracked: 1, conflicted: 0, total: 2 },
    reads: { models: [], tables: [], measures: [] },
  }),
}));

const { ProjectPanel } = await import("./ProjectPanel.js");

const STREAM = { id: "str1", kind: "primary", title: "Main", branch: "main" } as never;
const NOOP = async () => {};

afterEach(cleanup);

test("commit button count matches the header changed-count, not the file count", async () => {
  const view = render(
    <ProjectPanel
      stream={STREAM}
      vcsEnabled
      selectedFilePath={null}
      generated={[]}
      onOpenFile={() => {}}
      onCreateFile={NOOP}
      onCreateDirectory={NOOP}
      onRenamePath={NOOP}
      onDeletePath={NOOP}
      onToggleGenerated={NOOP}
    />,
  );

  const commit = await view.findByTestId("files-commit");
  await waitFor(() => expect(commit.textContent).toBe("Commit (2)"));
  // The header's "N changed" agrees with the button.
  expect(view.container.textContent).toContain("2 changed");
});
