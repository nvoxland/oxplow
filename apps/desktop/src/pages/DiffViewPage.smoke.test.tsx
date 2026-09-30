import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// P5.B4 (tsk523): the diff view runs on the neutral VCS surface alone —
// revisions, `diff`, `readAt`, `vcs*` — and never on a git-shaped command.
// Every command the page makes goes through a Proxy that records git-shaped
// names (and answers the rest with empty data), so a regression shows up as
// a recorded call rather than a swallowed promise rejection.

const realTransport = await import("../tauri-bridge/transport.js");
const realBindings = await import("../tauri-bridge/generated/bindings.js");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const GIT_SHAPED = /^(git(?!ChangeScopes|ResolveCommitRefLabels|ListRecentRemoteBranches)[A-Z]|get(BranchChanges|CommitDetail|RepoConflictState|WorkspaceStatusSummary|GitLog|CommitsAheadOf|AheadBehind|DefaultBranch)|list(Branches|LocalBranches|FileCommits|AllRefs|StreamDivergences|AdoptableWorktrees)|localBlame|readFile)/;
const sqlCalls: string[] = [];
const HEAD = "a".repeat(40);
const reads = { models: ["v_commit"], tables: [], measures: [] };
const gitCalls: string[] = [];
const diffCalls: unknown[][] = [];

const neutral: Record<string, (...args: unknown[]) => Promise<unknown>> = {
  diff: async (...args) => {
    diffCalls.push(args);
    return ok([{ path: "a.ts", status: "modified", additions: 2, deletions: 1 }]);
  },
  vcsRevision: async () =>
    ok({
      info: {
        id: "abc1234def",
        short_id: "abc1234",
        author: "Ann",
        email: "ann@example.com",
        time: 1_700_000_000,
        subject: "Fix the widget",
        parents: ["0001111"],
      },
      body: "",
      files: [{ path: "b.ts", status: "added", additions: 3, deletions: 0 }],
    }),
  vcsHead: async () => ok({ revision: `git:${HEAD}`, branch: "main" }),
  querySql: async (...args) => {
    const sql = String(args[0]);
    sqlCalls.push(sql);
    if (sql.includes("WITH RECURSIVE")) {
      return ok({
        columns: [],
        rows: [
          [HEAD, "Ann", "ann@example.com", "2026-09-30T00:00:00Z", "Second commit", `["${"b".repeat(40)}"]`],
          ["b".repeat(40), "Ann", "ann@example.com", "2026-09-29T00:00:00Z", "First commit", "[]"],
        ],
        truncated: false,
        reads,
        freshness: [],
      });
    }
    if (sql.includes("v_branch")) {
      return ok({ columns: [], rows: [["main", HEAD]], truncated: false, reads, freshness: [] });
    }
    return ok({ columns: [], rows: [], truncated: false, reads, freshness: [] });
  },
  listSnapshotsForStream: async () => ok([]),
  listEffortsOverlappingRange: async () => ok([]),
};

mock.module("../tauri-bridge/transport.js", () => ({
  ...realTransport,
  listen: async () => () => {},
}));

mock.module("../tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: new Proxy(
    {},
    {
      get(_target, name: string) {
        if (GIT_SHAPED.test(name)) {
          gitCalls.push(name);
          return async () => {
            throw new Error(`git-shaped command ${name}`);
          };
        }
        return neutral[name] ?? (async () => ok([]));
      },
    },
  ),
}));

const { DiffViewPage } = await import("./DiffViewPage.js");
const { GitCommitPage } = await import("./GitCommitPage.js");
const { HistoryPanel } = await import("../components/History/HistoryPanel.js");

afterEach(cleanup);

const STREAM = { id: "str1", kind: "primary", title: "Main", branch: "main" } as never;

test("the diff view lists a revision pair's files through the neutral surface only", async () => {
  const { findByText } = render(
    <DiffViewPage
      stream={STREAM}
      spec={{ mode: "endpoints", start: "git:abc1234", end: "working" }}
      onOpenPage={() => {}}
      onOpenFile={() => {}}
    />,
  );
  await findByText(/a\.ts/);
  await waitFor(() => expect(diffCalls.length).toBeGreaterThan(0));
  expect(diffCalls[0]).toEqual(["str1", "git:abc1234", "working"]);
  expect(gitCalls).toEqual([]);
});

test("the commit page reads its commit through vcsRevision, not git commands", async () => {
  gitCalls.length = 0;
  const { findByText } = render(
    <GitCommitPage
      stream={STREAM}
      sha="abc1234def"
      threadWork={null}
      onOpenPage={() => {}}
      onOpenFile={() => {}}
    />,
  );
  await findByText("Fix the widget");
  await findByText(/b\.ts/);
  expect(gitCalls).toEqual([]);
});

test("history reads v_commit from the stream's head, not git log", async () => {
  gitCalls.length = 0;
  const { findByText } = render(<HistoryPanel stream={STREAM} />);
  await findByText("Second commit");
  await findByText("First commit");
  expect(sqlCalls.some((sql) => sql.includes("WITH RECURSIVE"))).toBe(true);
  expect(gitCalls).toEqual([]);
});
