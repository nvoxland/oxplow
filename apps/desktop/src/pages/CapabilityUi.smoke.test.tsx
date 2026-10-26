import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";

// P6b.C6: the shell is complete with every enhancement off. Every command a
// page makes goes through a Proxy over the bindings answering empty data,
// as the page smoke tests do; then, with one extension, a replacement
// receives the params its target's contract lists.

const realTransport = await import("../tauri-bridge/transport.js");
const realBindings = await import("../tauri-bridge/generated/bindings.js");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const reads = { models: ["v_work_item"], tables: [], measures: [] };
const WORK_ITEM_COLUMNS = ["uri", "provider", "title", "body", "state", "parent_uri", "thread_id", "rank", "closed_at", "created_at", "updated_at", "native", "comment_count"];
let extensions: unknown[] = [];
let extensionLoads = 0;
const pageOpens: Array<[string, unknown]> = [];

const answers: Record<string, (...args: unknown[]) => Promise<unknown>> = {
  listExtensions: async () => {
    extensionLoads++;
    return ok(extensions);
  },
  diff: async () => ok([{ path: "a.ts", status: "modified", additions: 2, deletions: 1 }]),
  vcsRevision: async () =>
    ok({
      info: { id: "abc1234def", short_id: "abc1234", author: "Ann", email: "a@x", time: 1_700_000_000, subject: "Fix the widget", parents: [] },
      body: "",
      files: [{ path: "b.ts", status: "added", additions: 3, deletions: 0 }],
    }),
  querySql: async (...args) => {
    const sql = String(args[0]);
    if (sql.includes("head_sha")) {
      return ok({ columns: [], rows: [["a".repeat(40), "main"]], truncated: false, reads, freshness: [] });
    }
    if (sql.includes("FROM v_work_item w")) {
      return ok({
        columns: WORK_ITEM_COLUMNS,
        rows: [["oxplow:oxplow/work_item?id=W-1&provider=fake", "fake", "Their bug", "It breaks.", "todo", "oxplow:oxplow/work_item?id=W-0&provider=fake", null, null, null, "t", "t", null, 0]],
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
// answering these reads its own way — `realApi` is then the earlier mock,
// whose person commands would offer a Commands menu. Restore what the
// real `api` does — call the bindings (whichever bindings mock is current)
// and unwrap — so this file's answers apply here and a later file's
// bindings apply there.
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
  listPersonCommands: viaBindings("listPersonCommands"),
  querySql: (sql: string, params: unknown[] = [], limit: number | null = null, raw = false) =>
    viaBindings("querySql")(sql, params, limit, raw),
}));
mock.module("../tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: new Proxy({}, { get: (_t, name: string) => answers[name] ?? (async () => ok([])) }),
}));

const { PageNavigationContext } = await import("../tabs/PageNavigationContext.js");
const { Replaceable } = await import("../components/Replaceable.js");
const { decl, fakePage, fakePageRunsBy, fakeRun, loader, node } = await import("./testing/fakeRun.js");

/** The page runs a replacement opens: each page shows one row naming it. */
const runs = fakePageRunsBy((key) => {
  pageOpens.push([key.page, key.params]);
  const [extension, slug] = key.page.split("/") as [string, string];
  const page = fakePage(node.viz("rows", "rows", { columns: [{ key: "t" } as never] }), { id: key.page, extension, slug, title: "Mounted", loaders: [decl("rows")] });
  return fakeRun(page, { params: key.params, loaders: { rows: loader(["t"], [[`${key.page} row`]]) } });
}).runs;

afterEach(() => {
  extensions = [];
  extensionLoads = 0;
  pageOpens.length = 0;
  cleanup();
});

const STREAM = { id: "str1", kind: "primary", title: "Main", branch: "main" } as never;
/** `page` inside a navigation context for `ref`, as `App` mounts pages. */
function mount(ref: string, page: ReactElement) {
  const nav = { goBack() {}, goForward() {}, canGoBack: false, canGoForward: false, ask: { ref } };
  return render(<PageNavigationContext.Provider value={nav as never}>{page}</PageNavigationContext.Provider>);
}

/** Extension `name`, replacing `target` with its `page`. */
const replacing = (name: string, target: string, page: string, label: string) => ({
  name,
  enabled: true,
  ui: {
    slots: [],
    commands: [],
    decorators: [],
    replacements: [
      { id: `${name}/${target}`, extension: name, target, capability: "work_items", boundToProvider: false, page: `${name}/${page}`, label, version: 1, view: [] },
    ],
  },
});

// The Board is a view of the capability as a whole: an extension replaces
// it without bringing a provider, given the Board's props and nothing else;
// the page's own chrome stays oxplow's.
test("an extension's view replaces the Board, whichever list is active", async () => {
  extensions = [replacing("views", "work_item.board", "board", "board")];
  // What the Board's document places for its one replaceable part.
  const replaced = mount(
    "oxplow:oxplow/page?page=board",
    <Replaceable target="work_item.board" props={{ scope: "all", thread_id: null }} streamId="str1" runs={runs} onOpenPage={() => {}} fallback={<div data-testid="plain-board" />} />,
  );
  await waitFor(() => expect(replaced.getByTestId("replacement-work_item.board").textContent).toContain("replaced by views"));
  expect(replaced.queryByTestId("plain-board")).toBeNull();
  expect(pageOpens).toEqual([["views/board", { scope: "all", thread_id: null }]]);
});

// A work item's description editor is replaceable (the `body` part of the
// Work Item document); the document's other parts stay oxplow's.
test("an extension's editor replaces a work item's description", async () => {
  extensions = [replacing("views", "work_item.detail", "edit", "detail editor")];
  const uri = "oxplow:oxplow/work_item?id=W-1&provider=fake";
  const page = mount(uri, <Replaceable target="work_item.detail" props={{ uri }} streamId="str1" runs={runs} onOpenPage={() => {}} fallback={<div data-testid="plain-body" />} />);
  const replaced = await waitFor(() => page.getByTestId("replacement-work_item.detail"));
  expect(replaced.textContent).toContain("replaced by views");
  expect(pageOpens).toEqual([["views/edit", { uri }]]);
  expect(page.queryByTestId("plain-body")).toBeNull();
});

// provider's states (tsk918).
test("a work item's own provider's extension replaces its state control, not the active one's", async () => {
  const statePage = (name: string) => ({
    name,
    enabled: true,
    ui: {
      slots: [],
      commands: [],
      decorators: [],
      replacements: [
        { id: `${name}/work_item.detail.state`, extension: name, target: "work_item.detail.state", capability: "work_items", boundToProvider: true, page: `${name}/state`, label: "state control", version: 1, view: [] },
      ],
    },
  });
  extensions = [statePage("x"), statePage("y")];
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
  const uri = "oxplow:oxplow/work_item?id=W-1&provider=fake";
  const page = () =>
    mount(uri, <Replaceable target="work_item.detail.state" props={{ uri }} provider="fake" streamId="str1" runs={runs} onOpenPage={() => {}} fallback={<div data-testid="plain-state" />} />);
  try {
    const replaced = page();
    await waitFor(() => expect(replaced.getByTestId("replacement-work_item.detail.state").textContent).toContain("replaced by y"));
    expect(pageOpens).toEqual([["y/state", { uri }]]);
    cleanup();
    pageOpens.length = 0;

    // A provider no extension brings: oxplow's own state control alone.
    fakeExtension = null;
    const own = page();
    await waitFor(() => expect(own.getByTestId("plain-state")).toBeTruthy());
    expect(own.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
    expect(pageOpens).toEqual([]);
  } finally {
    answers.querySql = realQuery;
  }
});
