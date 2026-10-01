import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";

// P6b.C6: the shell is complete with every enhancement off — no
// extensions (so no slot sections, no Commands menu, no decorations) and
// every provider flag false (so no feature-gated action). Every command a
// page makes goes through a Proxy over the bindings answering empty data,
// as `DiffViewPage.smoke.test.tsx` does; then, with one extension, the
// mounts P6b added receive their params.

const realTransport = await import("../tauri-bridge/transport.js");
const realBindings = await import("../tauri-bridge/generated/bindings.js");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const reads = { models: ["v_work_item"], tables: [], measures: [] };
const WORK_ITEM_COLUMNS = ["ref", "provider", "title", "body", "state", "native_state", "parent_ref", "created_at", "updated_at", "task_id", "thread_id", "status", "priority", "sort_index", "author", "completed_at", "note_count"];
let extensions: unknown[] = [];
let extensionLoads = 0;
const lensRuns: Array<[string, unknown]> = [];

const answers: Record<string, (...args: unknown[]) => Promise<unknown>> = {
  listExtensions: async () => {
    extensionLoads++;
    return ok(extensions);
  },
  runLens: async (...args) => {
    lensRuns.push([String(args[0]), args[1]]);
    return ok({ lens: { id: args[0], title: "Mounted", columns: [], viz: "table", actions: [] }, params: args[1], result: { columns: [], rows: [], truncated: false, reads }, alert: null });
  },
  diff: async () => ok([{ path: "a.ts", status: "modified", additions: 2, deletions: 1 }]),
  vcsRevision: async () =>
    ok({
      info: { id: "abc1234def", short_id: "abc1234", author: "Ann", email: "a@x", time: 1_700_000_000, subject: "Fix the widget", parents: [] },
      body: "",
      files: [{ path: "b.ts", status: "added", additions: 3, deletions: 0 }],
    }),
  vcsHead: async () => ok({ revision: `git:${"a".repeat(40)}`, branch: "main" }),
  querySql: async (...args) => {
    const sql = String(args[0]);
    if (sql.includes("FROM v_work_item w")) {
      return ok({
        columns: WORK_ITEM_COLUMNS,
        rows: [["work_item:fake:W-1", "fake", "Their bug", "It breaks.", "todo", "Backlog", "work_item:fake:W-0", "t", "t", null, null, null, null, null, null, null, 0]],
        truncated: false,
        reads,
        freshness: [],
      });
    }
    return ok({ columns: [], rows: [], truncated: false, reads, freshness: [] });
  },
};

mock.module("../tauri-bridge/transport.js", () => ({ ...realTransport, listen: async () => () => {} }));
// Other test files mock `api.js` process-wide (the last mock wins), each
// answering these reads its own way. Restore what the real `api` does —
// call the bindings (whichever bindings mock is current) and unwrap — so
// this file's answers apply here and a later file's bindings apply there.
const realApi = await import("../api.js");
const viaBindings = (name: string) => async (...args: unknown[]) => {
  const { commands } = await import("../tauri-bridge/generated/bindings.js");
  const out = (await (commands as unknown as Record<string, (...a: unknown[]) => Promise<{ status: string; data?: unknown; error?: unknown }>>)[name]!(...args));
  if (out.status !== "ok") throw new Error(String(out.error));
  return out.data;
};
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: viaBindings("listExtensions"),
  querySql: (sql: string, params: unknown[] = [], limit: number | null = null, raw = false) =>
    viaBindings("querySql")(sql, params, limit, raw),
  runLens: viaBindings("runLens"),
}));
mock.module("../tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: new Proxy({}, { get: (_t, name: string) => answers[name] ?? (async () => ok([])) }),
}));

const { PageNavigationContext } = await import("../tabs/PageNavigationContext.js");
const { WorkItemPage } = await import("./WorkItemPage.js");
const { BoardPage } = await import("./BoardPage.js");
const { GitCommitPage } = await import("./GitCommitPage.js");
const { UncommittedChangesPage } = await import("./UncommittedChangesPage.js");
const { GitHistoryPage } = await import("./GitHistoryPage.js");

afterEach(() => {
  extensions = [];
  extensionLoads = 0;
  lensRuns.length = 0;
  cleanup();
});

const STREAM = { id: "str1", kind: "primary", title: "Main", branch: "main" } as never;
/** `page` inside a navigation context for `ref`, as `App` mounts pages. */
function mount(ref: string, page: ReactElement) {
  const nav = { goBack() {}, goForward() {}, canGoBack: false, canGoForward: false, ask: { ref, streamId: "str1" } };
  return render(<PageNavigationContext.Provider value={nav as never}>{page}</PageNavigationContext.Provider>);
}

/** Nothing an extension or a provider flag adds is on the page — checked
 *  once the page's extensions load has answered (and rendered). */
async function expectPlain(view: ReturnType<typeof render>) {
  await waitFor(() => expect(extensionLoads).toBeGreaterThan(0));
  await act(async () => {});
  // Any slot mount, whatever its slot (`LensSlots` marks each one).
  expect(view.container.querySelector("[data-slot]")).toBeNull();
  expect(view.queryByTestId("page-nav-commands")).toBeNull();
  expect(view.container.querySelector('[title^="from "]')).toBeNull();
}

test("another provider's item, with every flag off, is its core content and no more", async () => {
  const view = mount("work_item:fake:W-1", <WorkItemPage workItemRef="work_item:fake:W-1" streamId="str1" onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("work-item-page").textContent).toContain("It breaks."));
  expect(view.queryByTestId("work-item-comment-open")).toBeNull();
  expect(view.queryByTestId("work-item-link-open")).toBeNull();
  expect(view.queryByTestId("work-item-parent")).toBeNull();
  expect(view.getByTestId("work-item-move-done")).toBeTruthy();
  await expectPlain(view);
});

test("the Board, the commit page, uncommitted changes and history render plain", async () => {
  const board = mount("page:board", <BoardPage threadId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(board.getByTestId("work-board").textContent).toContain("Their bug"));
  await expectPlain(board);
  cleanup();

  const commit = mount("commit:abc1234def", <GitCommitPage stream={STREAM} sha="abc1234def" threadWork={null} onOpenPage={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(commit.container.textContent).toContain("Fix the widget"));
  await expectPlain(commit);
  cleanup();

  const uncommitted = mount("page:uncommitted-changes", <UncommittedChangesPage stream={STREAM} onOpenPage={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(uncommitted.getByTestId("uncommitted-files").textContent).toContain("a.ts"));
  await expectPlain(uncommitted);
  cleanup();

  const history = mount("page:git-history", <GitHistoryPage stream={STREAM} onOpenPage={() => {}} />);
  await waitFor(() => expect(history.getByTestId("page-git-history")).toBeTruthy());
  await expectPlain(history);
});

test("an extension's mounts reach the uncommitted strip and the history side column", async () => {
  extensions = [
    {
      name: "x",
      enabled: true,
      ui: {
        slots: [
          { slot: "vcs.status.header", lensId: "x/status" },
          { slot: "vcs.history.sidebar", lensId: "x/history" },
        ],
        commands: [],
        decorators: [],
      },
      lenses: ["x/status", "x/history"].map((id) => ({ id, params: [{ name: "stream_id", label: null, default: null }] })),
    },
  ];
  mount("page:uncommitted-changes", <UncommittedChangesPage stream={STREAM} onOpenPage={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(lensRuns).toContainEqual(["x/status", { stream_id: 1 }]));
  cleanup();
  const history = mount("page:git-history", <GitHistoryPage stream={STREAM} onOpenPage={() => {}} />);
  await waitFor(() => expect(lensRuns).toContainEqual(["x/history", { stream_id: 1 }]));
  await waitFor(() => expect(history.container.innerHTML).toContain('data-testid="vcs.history.sidebar-x/history"'));
});
