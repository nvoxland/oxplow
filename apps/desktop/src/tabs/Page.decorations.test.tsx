import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// P6b.C5: an enabled extension's decorator labels the page's ref with a
// chip after the page's own; a disabled one labels nothing.

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
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
        decorators: [{ id: "flags/0", extension: "flags", view: "v_flags_flags", resource: "oxplow/task", placement: "ref-chip", label: "label", color: "color" }],
      },
    },
    // A decorator on the request it was given on.
    {
      name: "audit",
      enabled: true, pages: [],
      ui: {
        slots: [],
        commands: [],
        decorators: [
          { id: "audit/0", extension: "audit", view: "v_audit_state", resource: "oxplow/request", placement: "ref-chip", label: "label", color: "color" },
        ],
      },
    },
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (sql.includes("FROM v_audit_state")) {
      return {
        columns: ["uri", "label", "color"],
        rows: [["oxplow:oxplow/request?id=req7", "Needs a look", "red"]],
        truncated: false,
        reads: { datasets: ["v_audit_state"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (!sql.includes("FROM v_flags_flags")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    queries.push(sql);
    return {
      columns: ["uri", "label", "color"],
      rows: [["oxplow:oxplow/task?id=W-1&provider=fake", "urgent", "#ff0000"]],
      truncated: false,
      reads: { datasets: ["v_flags_flags"], tables: [], measures: [] },
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
  ask: { ref: "oxplow:oxplow/task?id=W-1&provider=fake" },
} as unknown as PageNavigation;

const page = () =>
  render(
    <PageNavigationContext.Provider value={nav}>
      <Page title="Their bug" chips={[{ label: "todo" }]}>
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

// A request page, the request's ref in the navigation.
test("a request page shows its decorator as a chip", async () => {
  const requestNav = { ...nav, ask: { ref: "oxplow:oxplow/request?id=req7" } } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={requestNav}>
      <Page title="Changes" chips={[{ label: "closed" }]}>
        body
      </Page>
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(view.getByTestId("page-chips").textContent).toBe("closedNeeds a look"));
});

// `mock.module` is process-wide: put the real module back so the files that
// run after this one don't see this file's extensions.
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});
