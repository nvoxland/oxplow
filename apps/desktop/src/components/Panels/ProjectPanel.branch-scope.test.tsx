import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// The "branch" scope compares the working tree with where the branch
// forked. On a long branch that's thousands of files, and the daemon
// finishes every request it gets, so the panel must ask for paths only
// (`changedPaths`, never the line-counting `diffRevisions`) and must not
// start a new comparison for every file event.
//
// `api.js` is mocked directly (the reals spread) so the test owns event
// delivery whatever other files mocked into the shared module registry,
// and put back as it was afterwards so later files see their own mocks.

const realApi = { ...(await import("../../api.js")) };
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});

type File = { path: string; status: string | null };
let files: File[] = [
  { path: "a.ts", status: "modified" },
  { path: "b.ts", status: null },
];
const workspaceListeners = new Set<(e: { kind: string; path: string }) => void>();
const calls = { changedPaths: 0, diff: 0 };

mock.module("../../api.js", () => ({
  ...realApi,
  subscribeWorkspaceEvents: (_s: string, l: (e: { kind: string; path: string }) => void) => {
    workspaceListeners.add(l);
    return () => workspaceListeners.delete(l);
  },
  subscribeGitRefsEvents: () => () => {},
  // A fresh array each read, like the real index after an event.
  listWorkspaceFiles: async () => ({
    files: files.map((f) => ({ ...f })),
    summary: { modified: 1, added: 0, deleted: 0, untracked: 0, total: 1 },
  }),
  listWorkspaceEntries: async () => [],
  getChangeScopes: async () => ({
    current_branch: "feature",
    branch_base: "main",
    upstream: null,
    on_default_branch: false,
    staged: [],
    unstaged: [],
  }),
  vcsMergeBase: async () => "git:base",
  changedPaths: async () => {
    calls.changedPaths += 1;
    return files.filter((f) => f.status !== null).map((f) => ({ path: f.path, status: f.status }));
  },
  diffRevisions: async () => {
    calls.diff += 1;
    return [];
  },
}));

const { ProjectPanel } = await import("./ProjectPanel.js");

const STREAM = { id: "str1", kind: "worktree", title: "Feature", branch: "feature" } as never;
const NOOP = async () => {};
const settle = (ms: number) => new Promise((r) => setTimeout(r, ms));
const fileEvent = (path: string) =>
  act(() => {
    for (const l of workspaceListeners) l({ kind: "modified", path });
  });

afterEach(cleanup);

test("the branch scope asks for paths once per change to the changed set", async () => {
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
  fireEvent.click(await view.findByTestId("files-filter-toggle"));
  const branch = await view.findByTestId("files-filter-option-branch");
  await waitFor(() => expect((branch as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(branch);
  await waitFor(() => expect(calls.changedPaths).toBe(1));

  // Edits to a file that's already changed: the index reloads each time,
  // but the set of changed paths doesn't move, so neither does the scope.
  for (let i = 0; i < 10; i++) {
    fileEvent("a.ts");
    await settle(20);
  }
  await settle(400);
  expect(calls.changedPaths).toBe(1);

  // A newly changed file moves the set: one comparison for the burst.
  files = files.map((f) => (f.path === "b.ts" ? { ...f, status: "modified" } : f));
  for (let i = 0; i < 10; i++) {
    fileEvent("b.ts");
    await settle(20);
  }
  await waitFor(() => expect(calls.changedPaths).toBe(2));
  await settle(400);
  expect(calls.changedPaths).toBe(2);
  expect(calls.diff).toBe(0);
});
