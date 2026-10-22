import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensLink, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { CONTEXT_REF_MIME } from "../dragMimes.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// Lens rows can be grouped under headings (with actions there),
// styled from their own columns, and dragged into the agent's context.

const realApi = await import("../api.js");
const actionRuns: { action: string; row: unknown }[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  runLensAction: async (_id: string, action: string, _params: unknown, row: unknown) => {
    actionRuns.push({ action, row });
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { LensResultView } = await import("./LensResultView.js");

afterEach(cleanup);

const link = (kind: LensLink["kind"], from: string | null = null): LensLink => ({ kind, from, line: null, base: null, head: null });

const base: Lens = {
  id: "x/work",
  extension: "x",
  slug: "work",
  title: "Work",
  description: "",
  query: "",
  viz: "list",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
};
const columns = ["bucket", "bucket_ref", "id", "title", "status", "hue", "on", "d", "uri"];
const rows: SqlCell[][] = [
  ["In progress", "oxplow:oxplow/page?page=tasks", 1, "Epic", "in_progress", null, 0, 0, "oxplow:oxplow/work_item?id=tsk1&provider=oxplow"],
  ["In progress", "oxplow:oxplow/page?page=tasks", 2, "Child", "done", "warning", 1, 1, "oxplow:oxplow/work_item?id=tsk2&provider=oxplow"],
  ["Ready", null, 3, "Next", "todo", null, 0, 0, "oxplow:oxplow/work_item?id=tsk3&provider=oxplow"],
];
const runOf = (lens: Partial<Lens>): LensRun =>
  ({ lens: { ...base, ...lens }, params: {}, result: { columns, rows, truncated: false } }) as unknown as LensRun;
const grouped: Partial<Lens> = {
  group: { by: "bucket", link: link("page", "bucket_ref") },
  emphasis: "on",
  depth: "d",
  columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null, icon: "status", tone: "hue" }],
  actions: [{ id: "add", label: "+", command: "oxplow.work_item.create", input: {}, row: false, group: "Ready" }],
};

test("a grouped list puts its rows under a heading per value, in order, and the heading links", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const headings = view.getAllByTestId("lens-group-heading");
  expect(headings.map((h) => h.getAttribute("data-group"))).toEqual(["In progress", "Ready"]);
  expect(headings[0]!.querySelector("button")?.textContent).toBe("In progress");
  // "Ready" has no ref: plain text, not a link.
  expect(headings[1]!.querySelector("[data-testid=lens-group-link]")).toBeNull();
  // The group column isn't a cell.
  expect(view.getByTestId("lens-row-0").textContent).not.toContain("In progress");
  expect(view.getByTestId("lens-row-2").textContent).toContain("Next");
});

test("a group's action is a button in its heading, even compact, and runs without a row", async () => {
  actionRuns.length = 0;
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  expect(view.queryByTestId("lens-actions")).toBeNull();
  const button = view.getByTestId("lens-group-action-add");
  expect(button.closest("[data-group]")?.getAttribute("data-group")).toBe("Ready");
  fireEvent.click(button);
  await waitFor(() => expect(actionRuns).toEqual([{ action: "add", row: null }]));
});

test("a group action stays out of the toolbar", () => {
  const view = render(<LensResultView run={runOf(grouped)} onOpenPage={() => {}} />);
  expect(view.getByTestId("lens-actions").querySelector("[data-testid=lens-action-add]")).toBeNull();
  expect(view.getAllByTestId("lens-group-action-add")).toHaveLength(1);
});

test("cells draw their icon and tone; rows their emphasis and depth", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const child = view.getByTestId("lens-row-1");
  expect(child.querySelector("[data-testid=lens-icon]")?.getAttribute("aria-label")).toBe("Done");
  expect(child.querySelector("[data-tone=warning]")).not.toBeNull();
  expect(child.getAttribute("data-emphasis")).toBe("true");
  expect(view.getByTestId("lens-row-0").getAttribute("data-emphasis")).toBeNull();
  expect(child.style.paddingLeft).not.toBe(view.getByTestId("lens-row-0").style.paddingLeft);
});

test("a grouped table puts heading rows between its rows", () => {
  const view = render(<LensResultView run={runOf({ ...grouped, viz: "table" })} onOpenPage={() => {}} />);
  const table = view.getByTestId("lens-table");
  expect([...table.querySelectorAll("[data-testid=lens-group-heading]")].map((h) => h.getAttribute("data-group"))).toEqual([
    "In progress",
    "Ready",
  ]);
  expect([...table.querySelectorAll("th")].map((th) => th.textContent)).toContain("Title");
  expect([...table.querySelectorAll("thead th")].map((th) => th.textContent)).not.toContain("bucket");
});

function dragData(el: HTMLElement): Record<string, string> {
  const data: Record<string, string> = {};
  const dataTransfer = { setData: (k: string, v: string) => (data[k] = v), effectAllowed: "" };
  fireEvent.dragStart(el, { dataTransfer });
  return data;
}

test("a row that links somewhere drags into the agent's context; one that doesn't isn't draggable", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const row = view.getByTestId("lens-row-2");
  expect(row.getAttribute("draggable")).toBe("true");
  expect(JSON.parse(dragData(row)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "oxplow:oxplow/work_item?id=tsk3&provider=oxplow" });
  cleanup();
  const plain = render(<LensResultView run={runOf({})} onOpenPage={() => {}} />);
  expect(plain.getByTestId("lens-row-0").getAttribute("draggable")).toBeNull();
});

test("a tree's labels follow their column's link, and its rows drag", () => {
  // `d` names the parent: Child (d = 1) nests under Epic (id 1).
  const run = runOf({
    viz: "tree",
    tree: { id: "id", parent: "d", label: "title" },
    emphasis: "on",
    columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null, icon: "status", tone: null }],
  });
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  const nodes = view.getAllByTestId("lens-tree-row");
  // Its first button is the branch toggle; the label is the link.
  expect(nodes[0]!.querySelector("button:not([aria-expanded])")?.textContent).toContain("Epic");
  expect(nodes[0]!.querySelector("[data-testid=lens-icon]")).not.toBeNull();
  expect(JSON.parse(dragData(nodes[0]!)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" });
  expect(nodes.some((n) => n.getAttribute("data-emphasis") === "true")).toBe(true);
});

test("a lens that needs a capability that isn't active says so instead of showing rows", () => {
  const run = {
    ...runOf(grouped),
    inactive: { needs: ["work_items"], message: "Needs: Work list (choose one in Settings → Capabilities)." },
  } as LensRun;
  const view = render(<LensResultView run={run} compact onOpenPage={() => {}} />);
  expect(view.getByTestId("lens-inactive").textContent).toContain("Needs: Work list");
  expect(view.queryByTestId("lens-row-0")).toBeNull();
});

// A cell or heading that links to a ref opens it where its kind's page
// says (`openRef`), whatever the kind: a handoff, a note, an extension's.
test("a link to a ref of any kind opens through the page's openRef", () => {
  const opened: Array<[string, boolean | undefined]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { newTab?: boolean }) => opened.push([ref, how?.newTab]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView run={runOf(grouped)} compact />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.getByTestId("lens-row-2").querySelector("button")!);
  fireEvent.click(view.getAllByTestId("lens-group-link")[0]!, { metaKey: true });
  expect(opened).toEqual([
    ["oxplow:oxplow/work_item?id=tsk3&provider=oxplow", false],
    ["oxplow:oxplow/page?page=tasks", true],
  ]);
});

test("a highlighted column marks its param's value in the cell, as elements", () => {
  const run = {
    ...runOf({ columns: [{ key: "title", label: null, link: null, unit: null, highlight: "filter" }], params: [{ name: "filter", label: null, default: null, options: [] }] }),
    params: { filter: "ild" },
  } as unknown as LensRun;
  const view = render(<LensResultView run={run} compact onOpenPage={() => {}} />);
  const marks = view.getByTestId("lens-row-1").querySelectorAll("mark.ox-mark");
  expect(Array.from(marks).map((m) => m.textContent)).toEqual(["ild"]);
  expect(view.getByTestId("lens-row-1").textContent).toContain("Child");
  // No value: plain text.
  cleanup();
  const plain = render(<LensResultView run={{ ...run, params: { filter: "" } } as unknown as LensRun} compact onOpenPage={() => {}} />);
  expect(plain.container.querySelector("mark")).toBeNull();
});

/** A column's `select` makes a row a pick: clicking it (or Enter) sets
 *  that param to the row's value; a link inside it still only navigates,
 *  and the row holding the current value is drawn selected. */
test("a row click sets the param its column selects; a link in it doesn't", () => {
  const picked: [string, SqlCell][] = [];
  const run = {
    ...runOf({
      viz: "table",
      params: [{ name: "pick", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
      columns: [
        { key: "title", label: null, link: null, unit: null, icon: null, tone: null, select: "pick" },
        { key: "uri", label: null, link: link("page", "uri"), unit: null, icon: null, tone: null },
      ] as never,
    }),
    params: { pick: "Child" },
  } as LensRun;
  const view = render(<LensResultView run={run} onOpenPage={() => {}} onSetParam={(n, v) => picked.push([n, v])} />);
  fireEvent.click(view.getByTestId("lens-row-0"));
  fireEvent.keyDown(view.getByTestId("lens-row-2"), { key: "Enter" });
  const linkCell = view.getByTestId("lens-row-0").querySelectorAll("td")[1]!.firstElementChild as HTMLElement;
  fireEvent.click(linkCell);
  expect(picked).toEqual([
    ["pick", "Epic"],
    ["pick", "Next"],
  ]);
  expect(view.getByTestId("lens-row-1").getAttribute("aria-selected")).toBe("true");
  expect(view.getByTestId("lens-row-0").getAttribute("aria-selected")).toBe("false");
  cleanup();
  // With nowhere to set it, a row is no pick.
  const plain = render(<LensResultView run={run} onOpenPage={() => {}} />);
  expect(plain.getByTestId("lens-row-0").getAttribute("aria-selected")).toBeNull();
});

// The rail shows what was picked: the row as its columns label it.
test("a pick passes the row it came from, labelled as its columns are", () => {
  const rowsPicked: unknown[] = [];
  const run = {
    ...runOf({
      viz: "table",
      params: [{ name: "pick", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
      columns: [
        { key: "title", label: null, link: null, unit: null, icon: null, tone: null, select: "pick" },
        { key: "status", label: "State", link: null, unit: null, icon: null, tone: null },
      ] as never,
    }),
  } as LensRun;
  const view = render(<LensResultView run={run} onOpenPage={() => {}} onSetParam={(_n, _v, row) => rowsPicked.push(row)} />);
  fireEvent.click(view.getByTestId("lens-row-1"));
  expect(rowsPicked).toEqual([
    {
      value: "Child",
      facts: [
        { label: "Title", text: "Child" },
        { label: "State", text: "done" },
      ],
    },
  ]);
});

// A column with no label reads as words, not as its key.
test("a column with no label is headed by its key in words", () => {
  const view = render(<LensResultView run={runOf({ ...grouped, viz: "table" })} onOpenPage={() => {}} />);
  expect([...view.getByTestId("lens-table").querySelectorAll("thead th")].map((th) => th.textContent)).toContain("Title");
});

// On a page, Copy, Add to Agent Context and the lens's actions are the
// page's: handed up for its nav bar, not drawn above the body.
test("toolbarTo hands the toolbar's actions to the page instead of drawing them", async () => {
  let handed: { id: string; label: string }[] = [];
  const view = render(
    <LensResultView
      run={runOf({ actions: [{ id: "sync", label: "Sync", command: "oxplow.x", input: {}, row: false, group: null }] as never })}
      onOpenPage={() => {}}
      toolbarTo={(actions) => (handed = actions.map((a) => ({ id: a.id, label: a.label })))}
    />,
  );
  expect(view.queryByTestId("lens-actions")).toBeNull();
  await waitFor(() =>
    expect(handed).toEqual([
      { id: "lens-copy", label: "Copy" },
      { id: "lens-add-to-context", label: "Add to Agent Context" },
      { id: "lens-action-sync", label: "Sync" },
    ]),
  );
});

// A table's links are one collection: the page a row opens has Up and Down
// through the rows' pages (`Origin`).
test("a link opens with its column's pages as the origin", () => {
  const opened: Array<[string, unknown]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { origin?: unknown }) => opened.push([ref, how?.origin]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView
        run={runOf({ viz: "table", columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null }] })}
      />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.getByText("Child"));
  const [ref, origin] = opened[0]!;
  expect(ref).toBe("oxplow:oxplow/work_item?id=tsk2&provider=oxplow");
  expect((origin as { index: number; items: { label: string }[] }).index).toBe(1);
  expect((origin as { items: { label: string }[] }).items.map((i) => i.label)).toEqual(["Epic", "Child", "Next"]);
});

// A lens table is keyboard-driven as every list is (`useListNav`): ↑/↓ move
// the cursor, Enter opens the row's link, or runs its first row action when
// it has none.
test("↑/↓ and Enter open the row under the cursor", () => {
  const opened: Array<[string, unknown]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { origin?: unknown }) => opened.push([ref, how?.origin ? "origin" : "none"]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView
        run={runOf({ viz: "table", columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null }] })}
      />
    </PageNavigationContext.Provider>,
  );
  const table = view.getByTestId("lens-table");
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "ArrowUp" });
  fireEvent.keyDown(table, { key: "Enter" });
  expect(opened).toEqual([["oxplow:oxplow/work_item?id=tsk2&provider=oxplow", "origin"]]);
  expect(view.getByTestId("lens-row-1").getAttribute("data-cursor")).toBe("true");
});

test("Enter runs the first row action of a row with no link", async () => {
  actionRuns.length = 0;
  const view = render(
    <LensResultView
      run={runOf({
        viz: "table",
        columns: [{ key: "title", label: null, link: null, unit: null }],
        actions: [{ id: "open", label: "Open", command: "oxplow.work_item.update", input: {}, row: true }],
      })}
      onOpenPage={() => {}}
    />,
  );
  const table = view.getByTestId("lens-table");
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "Enter" });
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["open"]));
});

// A row action marked `primary` is what a click on the row runs (and Enter,
// before a link): the pointer's way to take a row.
test("a click on a row runs the primary action, but not from a link in it", async () => {
  actionRuns.length = 0;
  const view = render(
    <LensResultView
      run={runOf({
        viz: "table",
        columns: [{ key: "title", label: null, link: null, unit: null }],
        actions: [
          { id: "other", label: "Other", command: "oxplow.work_item.update", input: {}, row: true },
          { id: "take", label: "Take", command: "oxplow.work_item.update", input: {}, row: true, primary: true },
        ],
      })}
      onOpenPage={() => {}}
    />,
  );
  fireEvent.click(view.getByTestId("lens-row-1"));
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["take"]));
  actionRuns.length = 0;
  const table = view.getByTestId("lens-table");
  fireEvent.keyDown(table, { key: "Enter" });
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["take"]));
});
