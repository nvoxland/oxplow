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
let diffFiles: { path: string; status: string; additions: number; deletions: number }[] = [
  { path: "a.ts", status: "modified", additions: 2, deletions: 1 },
];
const extensionCalls: unknown[][] = [];
let extensionFailure: string | null = null;

const neutral: Record<string, (...args: unknown[]) => Promise<unknown>> = {
  diff: async (...args) => {
    diffCalls.push(args);
    return ok(diffFiles);
  },
  extensionEffectsBetween: async (...args) => {
    extensionCalls.push(args);
    if (extensionFailure) throw new Error(extensionFailure);
    return ok([
      {
        name: "acme",
        change: "changed",
        errors: [],
        effects: {
          lenses: [{ id: "acme/count", change: "changed", before: "1", after: "2", error: null }],
          models: [],
          collectors: [],
          providers: [],
          effects: [],
          config: null,
          lines: ["Lens acme/count: changed"],
        },
      },
    ]);
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

// P8.C7: an effort's review shows "Extension Changes" only when files under
// `oxplow/extensions/` changed — what each change does, in the server's
// lines, with a changed lens's text before and after.
test("extension changes show only when an extension's files changed", async () => {
  const first = render(
    <DiffViewPage stream={STREAM} spec={{ mode: "endpoints", start: "snap:1", end: "snap:2" }} onOpenPage={() => {}} onOpenFile={() => {}} />,
  );
  await first.findByText(/a\.ts/);
  expect(first.queryByTestId("diff-view-extension-changes")).toBeNull();
  expect(extensionCalls).toEqual([]);
  cleanup();

  diffFiles = [{ path: "oxplow/extensions/acme/lenses/count.yaml", status: "modified", additions: 1, deletions: 1 }];
  const second = render(
    <DiffViewPage stream={STREAM} spec={{ mode: "endpoints", start: "snap:1", end: "snap:2" }} onOpenPage={() => {}} onOpenFile={() => {}} />,
  );
  const lines = await second.findByTestId("extension-change-acme-effects");
  expect(lines.textContent).toContain("Lens acme/count: changed");
  expect(second.getByTestId("effect-lens-acme/count")).toBeTruthy();
  expect(extensionCalls[0]).toEqual(["str1", "snap:1", "snap:2"]);
  diffFiles = [{ path: "a.ts", status: "modified", additions: 2, deletions: 1 }];
});

// tsk793: a review that fails says so, instead of "Reviewing…" forever.
test("a failed extension review shows its error", async () => {
  diffFiles = [{ path: "oxplow/extensions/acme/lenses/count.yaml", status: "modified", additions: 1, deletions: 1 }];
  extensionFailure = "no such revision";
  try {
    const page = render(
      <DiffViewPage stream={STREAM} spec={{ mode: "endpoints", start: "snap:1", end: "snap:2" }} onOpenPage={() => {}} onOpenFile={() => {}} />,
    );
    const error = await page.findByTestId("diff-view-extension-changes-error");
    expect(error.textContent).toContain("Could not review acme");
    expect(error.textContent).toContain("no such revision");
    expect(page.queryByText(/Reviewing/)).toBeNull();
  } finally {
    extensionFailure = null;
    diffFiles = [{ path: "a.ts", status: "modified", additions: 2, deletions: 1 }];
  }
});
