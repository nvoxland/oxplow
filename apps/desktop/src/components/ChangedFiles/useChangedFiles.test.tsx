import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

// The working-tree list is live: file events refresh it. The daemon
// finishes every diff it's asked for, so a burst of events must coalesce
// into one follow-up diff, not one per event.
//
// `api.js` is mocked directly (the reals spread) so the test owns event
// delivery whatever other files mocked into the shared module registry,
// and put back as it was afterwards so later files see their own mocks.

const realApi = { ...(await import("../../api.js")) };
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});

const workspaceListeners = new Set<() => void>();
let diffs = 0;

mock.module("../../api.js", () => ({
  ...realApi,
  subscribeWorkspaceEvents: (_s: string, l: () => void) => {
    workspaceListeners.add(l);
    return () => workspaceListeners.delete(l);
  },
  subscribeGitRefsEvents: () => () => {},
  subscribeSnapshotEvents: () => () => {},
  vcsHead: async () => ({ revision: "git:head", branch: "main" }),
  diffRevisions: async () => {
    diffs += 1;
    return [{ path: "a.ts", status: "modified", additions: 1, deletions: 0 }];
  },
}));

const { useChangedFiles } = await import("./useChangedFiles.js");

afterEach(cleanup);

test("a burst of file events refreshes the working-tree list once", async () => {
  const source = { kind: "working" as const, streamId: "str1" };
  const { result } = renderHook(() => useChangedFiles(source));
  await waitFor(() => expect(result.current.files.length).toBe(1));
  expect(diffs).toBe(1);

  for (let i = 0; i < 10; i++) {
    act(() => {
      for (const l of workspaceListeners) l();
    });
    await new Promise((r) => setTimeout(r, 20));
  }
  await waitFor(() => expect(diffs).toBe(2));
  await new Promise((r) => setTimeout(r, 400));
  expect(diffs).toBe(2);
});
