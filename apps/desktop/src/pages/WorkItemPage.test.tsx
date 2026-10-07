import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P6b.C3: another provider's work item has a page of its own, and what it
// offers follows the provider's declared features.

const realApi = await import("../api.js");
const realQuerySql = realApi.querySql;
const realRunCommand = realApi.runCommand;
let features: Record<string, boolean> = {};
let parent: string | null = "work_item:fake:W-0";
const ran: Array<[string, unknown]> = [];
const lensRuns: Array<[string, unknown]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (sql.includes("FROM v_capability_provider")) {
      return {
        columns: ["capability", "provider", "extension", "features", "active"],
        rows: [["work_items", "fake", "tracker", JSON.stringify(features), 1]],
        truncated: false,
        reads: { models: ["v_capability_provider"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_work_item w")) {
      return {
        columns: ["ref", "provider", "title", "body", "state", "native_state", "parent_ref", "created_at", "updated_at", "task_id", "thread_id", "status", "priority", "sort_index", "author", "completed_at", "note_count"],
        rows: [["work_item:fake:W-1", "fake", "Their bug", "It breaks.", "todo", "Backlog", parent, "t", "t", null, null, null, null, null, null, null, 0]],
        truncated: false,
        reads: { models: ["v_work_item"], tables: [], measures: [] },
        freshness: {},
      };
    }
    return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
  },
  runCommand: async (name: string, input: unknown, ...rest: unknown[]) => {
    if (!name.startsWith("oxplow.work_item.")) return (realRunCommand as (...a: unknown[]) => unknown)(name, input, ...rest);
    ran.push([name, rest[0] === true ? { ...(input as object), confirmed: true } : input]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      ui: {
        slots: [
          { slot: "work_item.detail.body", lensId: "x/body" },
          { slot: "work_item.detail.sidebar", lensId: "x/side" },
        ],
        commands: [],
        decorators: [],
        replacements: [],
      },
      lenses: ["x/body", "x/side"].map((id) => ({ id, params: [{ name: "ref", label: null, default: null }] })),
    },
  ],
  runLens: async (id: string, params: unknown) => {
    lensRuns.push([id, params]);
    return { lens: { id, title: id, columns: [], viz: "table", actions: [] }, params, result: { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] } }, alert: null };
  },
}));
const { WorkItemPage } = await import("./WorkItemPage.js");
const { PageNavigationContext } = await import("../tabs/PageNavigationContext.js");

afterEach(() => {
  parent = "work_item:fake:W-0";
  ran.length = 0;
  lensRuns.length = 0;
  cleanup();
});

const page = () => render(<WorkItemPage workItemRef="work_item:fake:W-1" streamId={null} onOpenPage={() => {}} />);

test("a provider without comments, links or hierarchy offers none of them", async () => {
  features = {};
  const view = page();
  await waitFor(() => expect(view.getByTestId("work-item-page").textContent).toContain("Their bug"));
  expect(view.getByTestId("work-item-page").textContent).toContain("It breaks.");
  expect(view.queryByTestId("work-item-comment-open")).toBeNull();
  expect(view.queryByTestId("work-item-link-open")).toBeNull();
  expect(view.queryByTestId("work-item-parent")).toBeNull();
  expect(view.queryByTestId("work-item-delete-trigger")).toBeNull();
});

// P7.A1: Delete shows only when the provider declares `delete`; the
// inline confirm is the person's confirmation.
test("Delete shows with the provider's delete feature and runs oxplow.work_item.delete confirmed", async () => {
  features = { delete: true };
  const view = page();
  fireEvent.click(await waitFor(() => view.getByTestId("work-item-delete-trigger")));
  fireEvent.click(view.getByTestId("work-item-delete-confirm"));
  await waitFor(() => expect(ran).toEqual([["oxplow.work_item.delete", { ref: "work_item:fake:W-1", confirmed: true }]]));
});

// P7.A1: every write is a `work_item.*` command, whatever the provider.
test("comments, links and a parent show when the provider declares them; Comment runs oxplow.work_item.comment", async () => {
  features = { comments: true, links: true, hierarchy: true };
  const view = page();
  const open = await waitFor(() => view.getByTestId("work-item-comment-open"));
  expect(view.getByTestId("work-item-link-open")).toBeTruthy();
  expect(view.getByTestId("work-item-parent").textContent).toContain("work_item:fake:W-0");
  fireEvent.click(open);
  fireEvent.change(view.getByTestId("work-item-comment-body"), { target: { value: "Seen it too." } });
  fireEvent.keyDown(view.getByTestId("work-item-comment-body"), { key: "Enter", metaKey: true });
  await waitFor(() =>
    expect(ran).toEqual([["oxplow.work_item.comment", { ref: "work_item:fake:W-1", body: "Seen it too." }]]),
  );
  await waitFor(() => expect(view.queryByTestId("work-item-comment-body")).toBeNull());
});

test("Link… takes any link type the provider names, not oxplow's list", async () => {
  features = { links: true };
  const view = page();
  fireEvent.click(await waitFor(() => view.getByTestId("work-item-link-open")));
  expect((view.getByTestId("work-item-link-link_type") as HTMLInputElement).value).toBe("relates_to");
  fireEvent.change(view.getByTestId("work-item-link-link_type"), { target: { value: "caused_by" } });
  fireEvent.change(view.getByTestId("work-item-link-target"), { target: { value: "work_item:fake:W-9" } });
  fireEvent.click(view.getByTestId("work-item-link-submit"));
  await waitFor(() =>
    expect(ran).toEqual([["oxplow.work_item.link", { ref: "work_item:fake:W-1", target: "work_item:fake:W-9", link_type: "caused_by" }]]),
  );
});

test("the tab is titled with the item's title", async () => {
  features = {};
  const titles: string[] = [];
  const nav = { goBack() {}, goForward() {}, canGoBack: false, canGoForward: false, setTitle: (t: string) => titles.push(t) };
  render(
    <PageNavigationContext.Provider value={nav as never}>
      <WorkItemPage workItemRef="work_item:fake:W-1" streamId={null} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(titles).toContain("Their bug"));
});

test("Move To runs the provider's transition; the item's slots get its ref", async () => {
  features = {};
  const view = page();
  fireEvent.click(await waitFor(() => view.getByTestId("work-item-move-done")));
  await waitFor(() => expect(ran).toEqual([["oxplow.work_item.transition", { ref: "work_item:fake:W-1", to: "done" }]]));
  await waitFor(() =>
    expect(lensRuns.map(([id, p]) => [id, p])).toEqual(
      expect.arrayContaining([
        ["x/body", { ref: "work_item:fake:W-1" }],
        ["x/side", { ref: "work_item:fake:W-1" }],
      ]),
    ),
  );
});

test("a provider with hierarchy shows no Parent for an item without one", async () => {
  features = { hierarchy: true };
  parent = null;
  const view = page();
  await waitFor(() => expect(view.getByTestId("work-item-page").textContent).toContain("Their bug"));
  expect(view.queryByTestId("work-item-parent")).toBeNull();
});
