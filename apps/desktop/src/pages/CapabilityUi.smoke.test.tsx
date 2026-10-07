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
const WORK_ITEM_COLUMNS = ["ref", "provider", "title", "body", "state", "parent_ref", "thread_id", "rank", "closed_at", "created_at", "updated_at", "native", "comment_count"];
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
        rows: [["work_item:fake:W-1", "fake", "Their bug", "It breaks.", "todo", "work_item:fake:W-0", null, null, null, "t", "t", null, 0]],
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
const { DiffFileHeaderSlot } = await import("../components/Diff/DiffFileHeaderSlot.js");

afterEach(() => {
  extensions = [];
  extensionLoads = 0;
  lensRuns.length = 0;
  cleanup();
});

const STREAM = { id: "str1", kind: "primary", title: "Main", branch: "main" } as never;
/** `page` inside a navigation context for `ref`, as `App` mounts pages. */
function mount(ref: string, page: ReactElement) {
  const nav = { goBack() {}, goForward() {}, canGoBack: false, canGoForward: false, ask: { ref } };
  return render(<PageNavigationContext.Provider value={nav as never}>{page}</PageNavigationContext.Provider>);
}

/** Nothing an extension or a provider flag adds is on the page — checked
 *  once the page's extensions load has answered (and rendered). */
async function expectPlain(view: ReturnType<typeof render>) {
  await waitFor(() => expect(extensionLoads).toBeGreaterThan(0));
  await act(async () => {});
  // Any slot mount, whatever its slot (`LensSlots` marks each one).
  expect(view.container.querySelector("[data-slot]")).toBeNull();
  // No core component replaced (`Replaceable` marks one that is).
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
  expect(view.queryByTestId("page-nav-commands")).toBeNull();
  expect(view.container.querySelector('[title^="from "]')).toBeNull();
}

test("another provider's item, with every flag off, is its core content and no more", async () => {
  const view = mount("work_item:fake:W-1", <WorkItemPage workItemRef="work_item:fake:W-1" stream={null} thread={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("work-item-page").textContent).toContain("It breaks."));
  expect(view.queryByTestId("work-item-comment-open")).toBeNull();
  expect(view.queryByTestId("work-item-link-open")).toBeNull();
  expect(view.queryByTestId("work-item-parent")).toBeNull();
  expect(view.queryByTestId("task-rail-delete-trigger")).toBeNull();
  expect(view.getByLabelText("State")).toBeTruthy();
  await expectPlain(view);
});

test("the Board, the commit page, uncommitted changes and history render plain", async () => {
  const board = mount("page:board", <BoardPage threadId={null} streamId="str1" onOpenPage={() => {}} />);
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
  // Mounted, with no rows: it folds into the column's "Nothing found" line,
  // named (tsk1036).
  await waitFor(() => expect(history.getByTestId("vcs.history.sidebar-nothing-found").textContent).toContain("Mounted"));
});


// P9.A2: a file diff's header strip — plain with nothing mounted; a
// mounted lens gets the path and the two revisions it declares.
test("the file diff's header strip is plain, and a mount gets the file and its revisions", async () => {
  const spec = { path: "a.ts", leftVersion: "git:abc", rightVersion: "working", baseLabel: "HEAD" } as never;
  const plain = mount("page:diff", <DiffFileHeaderSlot streamId="str1" spec={spec} />);
  await expectPlain(plain);
  cleanup();

  answers.runLens = async (...args) => {
    lensRuns.push([String(args[0]), args[1]]);
    return ok({ lens: { id: args[0], title: "Usually Changes With", columns: [], viz: "table", actions: [] }, params: args[1], result: { columns: ["path"], rows: [["b.ts"]], truncated: false, reads }, alert: null });
  };
  extensions = [
    {
      name: "x",
      enabled: true,
      ui: { slots: [{ slot: "diff.file.header", lensId: "x/co" }], commands: [], decorators: [] },
      lenses: [{ id: "x/co", params: ["path", "left_revision", "right_revision"].map((name) => ({ name, label: null, default: null })) }],
    },
  ];
  const view = mount("page:diff", <DiffFileHeaderSlot streamId="str1" spec={spec} />);
  await waitFor(() =>
    expect(lensRuns).toContainEqual(["x/co", { path: "a.ts", left_revision: "git:abc", right_revision: "working" }]),
  );
  await waitFor(() => expect(view.container.innerHTML).toContain('data-testid="diff.file.header-x/co"'));
});

// P9.A1: the Board is replaceable — by the active work-items provider's
// extension only, given the Board's props and nothing else.
test("the active provider's extension replaces the Board; another's doesn't", async () => {
  const replacing = (name: string) => ({
    name,
    enabled: true,
    ui: {
      slots: [],
      commands: [],
      decorators: [],
      replacements: [{ id: `${name}/work_item.board`, extension: name, target: "work_item.board", capability: "work_items", lensId: `${name}/board`, label: "board" }],
    },
    lenses: [{ id: `${name}/board`, params: ["scope", "thread_id"].map((p) => ({ name: p, label: null, default: null })) }],
  });
  extensions = [replacing("x"), replacing("y")];
  const realQuery = answers.querySql!;
  let active = "oxplow";
  answers.querySql = async (...args) => {
    if (!String(args[0]).includes("v_capability_provider")) return realQuery(...args);
    return ok({
      columns: ["capability", "provider", "extension", "features", "active"],
      rows: [
        ["work_items", "oxplow", null, "{}", active === "oxplow" ? 1 : 0],
        ["work_items", "fake", "x", "{}", active === "fake" ? 1 : 0],
        ["work_items", "other", "y", "{}", 0],
      ],
      truncated: false,
      reads: { models: ["v_capability_provider"], tables: [], measures: [] },
      freshness: [],
    });
  };
  try {
    // Installed but not active: oxplow's own Board, and no replacement lens runs.
    const own = mount("page:board", <BoardPage threadId={null} streamId="str1" onOpenPage={() => {}} />);
    await waitFor(() => expect(own.getByTestId("work-board").textContent).toContain("Their bug"));
    expect(own.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
    expect(lensRuns).toEqual([]);
    cleanup();

    active = "fake";
    const replaced = mount("page:board", <BoardPage threadId={null} streamId="str1" onOpenPage={() => {}} />);
    await waitFor(() => expect(replaced.getByTestId("replacement-work_item.board").textContent).toContain("replaced by x"));
    expect(replaced.queryByTestId("work-board")).toBeNull();
    expect(lensRuns).toEqual([["x/board", { scope: "all", thread_id: null }]]);
    // The page's own chrome — its scope picker — is still oxplow's.
    expect(replaced.getByTestId("board-scope")).toBeTruthy();
  } finally {
    answers.querySql = realQuery;
  }
});


// P10 (K4): a work item's state control (State and Move To) is the second
// replaceable component, given the item's ref and nothing else. Which
// extension replaces it is the **item's own provider's**, not the active
// one's: another provider's item never gets a lens that sends that
// provider's states (tsk918).
test("a work item's own provider's extension replaces its state control, not the active one's", async () => {
  const stateLens = (name: string) => ({
    name,
    enabled: true,
    ui: {
      slots: [],
      commands: [],
      decorators: [],
      replacements: [
        { id: `${name}/work_item.detail.state`, extension: name, target: "work_item.detail.state", capability: "work_items", lensId: `${name}/state`, label: "state control" },
      ],
    },
    lenses: [{ id: `${name}/state`, params: [{ name: "ref", label: null, default: null }] }],
  });
  extensions = [stateLens("x"), stateLens("y")];
  const realQuery = answers.querySql!;
  // The item is `fake`'s; `lin` (extension x) is active.
  let fakeExtension: string | null = "y";
  answers.querySql = async (...args) => {
    if (!String(args[0]).includes("v_capability_provider")) return realQuery(...args);
    return ok({
      columns: ["capability", "provider", "extension", "features", "fields", "id_pattern", "active"],
      rows: [
        ["work_items", "oxplow", null, "{}", "[]", null, 0],
        ["work_items", "lin", "x", "{}", "[]", null, 1],
        ["work_items", "fake", fakeExtension, "{}", "[]", null, 0],
      ],
      truncated: false,
      reads: { models: ["v_capability_provider"], tables: [], measures: [] },
      freshness: [],
    });
  };
  const page = () => mount("work_item:fake:W-1", <WorkItemPage workItemRef="work_item:fake:W-1" stream={null} thread={null} onOpenPage={() => {}} />);
  try {
    const replaced = page();
    await waitFor(() => expect(replaced.getByTestId("replacement-work_item.detail.state").textContent).toContain("replaced by y"));
    // Oxplow's state control isn't the replacement's to take: whatever
    // states the lens offers, the item can always move.
    expect(replaced.getByLabelText("State")).toBeTruthy();
    expect(replaced.getByTestId("replacement-work_item.detail.state").contains(replaced.getByLabelText("State"))).toBe(false);
    expect(lensRuns).toEqual([["y/state", { ref: "work_item:fake:W-1" }]]);
    // The rest of the page is still oxplow's.
    expect(replaced.getByTestId("work-item-page").textContent).toContain("It breaks.");
    cleanup();
    lensRuns.length = 0;

    // A provider no extension brings: oxplow's own state control alone.
    fakeExtension = null;
    const own = page();
    await waitFor(() => expect(own.getByLabelText("State")).toBeTruthy());
    expect(own.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
    expect(lensRuns).toEqual([]);
  } finally {
    answers.querySql = realQuery;
  }
});
