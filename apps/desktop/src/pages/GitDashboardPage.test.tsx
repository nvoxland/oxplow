import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// tsk899: each Git Dashboard action is wired through its command's spec —
// one that asks arms on the first click and runs only on the confirm, one
// that doesn't runs on the click.

const realApi = await import("../api.js");
const ran: string[] = [];
const kickoff = { taskId: "t1", await: async () => ({ success: true, log: "", conflicts: [], auto_resolved: [] }) };
const stream = { id: "str1", title: "Main", branch: "main" };
const other = { id: "str2", title: "Feature", branch: "feature" };
mock.module("../api.js", () => ({
  ...realApi,
  getCommand: async (name: string) => ({
    name,
    summary: name,
    confirm: name === "vcs.merge" || name === "git.rebase" ? "destructive" : "never",
  }),
  vcsHead: async () => ({ branch: "main", revision: "git:abc", detached: false }),
  vcsStatus: async () => ({ entries: [] }),
  countStatus: () => ({ added: 0, modified: 0, deleted: 0, untracked: 0, conflicted: 0, total: 0 }),
  listRecentRemoteBranches: async () => [
    { short_name: "origin/main", last_commit_subject: "s", last_commit_at: "2026-01-01T00:00:00Z" },
  ],
  listStreams: async () => [stream, other],
  vcsDivergence: async () => ({ ahead: 2, behind: 1, overlapping_files: [], readiness: "clean" }),
  vcsRevision: async () => null,
  vcsRevisionsBetween: async () => [],
  listAgentStatuses: async () => [],
  subscribeAgentStatus: () => () => {},
  subscribeGitRefsEvents: () => () => {},
  subscribeWorkspaceEvents: () => () => {},
  vcsPush: async () => {
    ran.push("push");
    return kickoff;
  },
  vcsPull: async () => {
    ran.push("pull");
    return kickoff;
  },
  vcsFetch: async () => {
    ran.push("fetch");
    return kickoff;
  },
  vcsMerge: async (_s: string, rev: string) => {
    ran.push(`merge ${rev}`);
    return kickoff;
  },
  gitRebase: async (_s: string, rev: string) => {
    ran.push(`rebase ${rev}`);
    return kickoff;
  },
}));
mock.module("../vcsHistory.js", () => ({
  readHistory: async () => ({
    commits: [],
    branchHeads: [],
    tags: [],
    reads: { models: [], tables: [], measures: [] },
  }),
  readBranches: async () => ({
    branches: [{ name: "main", remote: null, isDefault: true }],
    reads: { models: [], tables: [], measures: [] },
  }),
}));
mock.module("../git-op.js", () => ({
  awaitGitOp: async (k: typeof kickoff) => k.await(),
  opErrorOf: (label: string) => ({ label }),
}));

const { GitDashboardPage } = await import("./GitDashboardPage.js");

afterEach(() => {
  cleanup();
  ran.length = 0;
});

function page() {
  return render(
    <GitDashboardPage stream={stream as never} onOpenPage={() => {}} onRevealCommit={() => {}} />,
  );
}

test("push doesn't ask: one click runs it", async () => {
  const view = page();
  await waitFor(() => view.getByTestId("git-dashboard-push"));
  // Its spec says `never` once loaded (until then it asks).
  await new Promise((r) => setTimeout(r, 30));
  fireEvent.click(view.getByTestId("git-dashboard-push"));
  await waitFor(() => expect(ran).toEqual(["push"]));
  expect(view.queryByTestId("git-dashboard-push-confirm")).toBeNull();
});

test("a stream's merge asks: the first click arms, the confirm runs", async () => {
  const view = page();
  const merge = await waitFor(() => view.getByTestId("git-dashboard-stream-merge-rebase"));
  await new Promise((r) => setTimeout(r, 30));
  fireEvent.click(merge);
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("git-dashboard-stream-merge-rebase-confirm"));
  await waitFor(() => expect(ran).toEqual(["merge feature"]));
});
