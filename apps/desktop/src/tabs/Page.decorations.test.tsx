import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// P6b.C5: an enabled extension's decorator labels the page's ref with a
// chip after the page's own; a disabled one labels nothing.

const realApi = await import("../api.js");
const realQuerySql = realApi.querySql;
let enabled = true;
const queries: string[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "flags",
      enabled,
      ui: {
        slots: [],
        commands: [],
        decorators: [{ id: "flags/0", extension: "flags", view: "v_flags_flags", kind: "work_item", placement: "ref-chip", label: "label", color: "color" }],
      },
    },
    // P10 (K1): oxplow-bundled's verdict, on the effort it was given on.
    {
      name: "oxplow-bundled",
      enabled: true,
      ui: {
        slots: [],
        commands: [],
        decorators: [
          { id: "oxplow-bundled/0", extension: "oxplow-bundled", view: "v_oxplow_bundled_verdict", kind: "effort", placement: "ref-chip", label: "label", color: "color" },
        ],
      },
    },
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (sql.includes("FROM v_oxplow_bundled_verdict")) {
      return {
        columns: ["ref", "label", "color"],
        rows: [["effort:eff7", "Changes requested", "red"]],
        truncated: false,
        reads: { models: ["v_oxplow_bundled_verdict"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (!sql.includes("FROM v_flags_flags")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    queries.push(sql);
    return {
      columns: ["ref", "label", "color"],
      rows: [["work_item:fake:W-1", "urgent", "#ff0000"]],
      truncated: false,
      reads: { models: ["v_flags_flags"], tables: [], measures: [] },
      freshness: {},
    };
  },
}));
const { Page } = await import("./Page.js");

afterEach(() => {
  queries.length = 0;
  cleanup();
});

const nav = {
  goBack() {},
  goForward() {},
  canGoBack: false,
  canGoForward: false,
  ask: { ref: "work_item:fake:W-1", streamId: null },
} as unknown as PageNavigation;

const page = () =>
  render(
    <PageNavigationContext.Provider value={nav}>
      <Page title="Their bug" kind="work_item" chips={[{ label: "todo" }]}>
        body
      </Page>
    </PageNavigationContext.Provider>,
  );

test("a decorator's label is a chip on the page's ref, after the page's own", async () => {
  enabled = true;
  const view = page();
  await waitFor(() => expect(view.getByTestId("page-chips").textContent).toBe("todourgent"));
  expect(queries).toHaveLength(1);
});

test("a disabled extension decorates nothing", async () => {
  enabled = false;
  const view = page();
  await new Promise((r) => setTimeout(r, 30));
  expect(view.getByTestId("page-chips").textContent).toBe("todo");
  expect(queries).toHaveLength(0);
});

// An effort opens as its diff, the effort's ref in the navigation.
test("an effort page shows its review verdict as a chip", async () => {
  const effortNav = { ...nav, ask: { ref: "effort:eff7", streamId: null } } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={effortNav}>
      <Page title="Changes" kind="diff-view" chips={[{ label: "closed" }]}>
        body
      </Page>
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(view.getByTestId("page-chips").textContent).toBe("closedChanges requested"));
});
