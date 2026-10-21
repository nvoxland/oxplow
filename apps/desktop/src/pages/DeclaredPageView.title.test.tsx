import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Extension, ExtensionPage, LensRun } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A declared page's tab says the page's manifest title — what the
// launcher showed — not its id, and not its lens's title.

const realApi = await import("../api.js");
// Bun's `mock.module` is process-wide: only what this page reads is faked.
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () =>
    [{ name: "acme", enabled: true, ui: { slots: [], commands: [], decorators: [] }, pages: [] }] as unknown as Extension[],
  runLens: async (): Promise<LensRun> =>
    ({
      lens: { id: "acme/prs", title: "PRs (lens)", viz: "table", params: [], columns: [], actions: [], children: [], parts: [], origin: { extension: "acme", source: null, running: "", deployment: null, bundled: false } },
      params: {},
      result: { columns: ["n"], rows: [[1]], truncated: false, reads: { models: [], tables: [], measures: [] } },
      alert: null,
    }) as unknown as LensRun,
  subscribeOxplowEvents: () => () => {},
}));
const { DeclaredPageView } = await import("./DeclaredPageView.js");

const declared = {
  id: "open-prs",
  extension: "acme",
  pageRef: "oxplow:oxplow/page?page=acme/open-prs",
  title: "Open Pull Requests",
  icon: null,
  category: "Work",
  keywords: null,
  params: [],
  count: null,
  body: { kind: "lens", lens: "acme/prs" },
} as unknown as ExtensionPage;

afterEach(cleanup);

test("the tab title is the page's manifest title", async () => {
  const titles: string[] = [];
  const nav = {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    setTitle: (t: string) => titles.push(t),
  } as unknown as PageNavigation;
  render(
    <PageNavigationContext.Provider value={nav}>
      <DeclaredPageView page={{ extension: "acme", page: "open-prs", params: {} }} declared={declared} stream={null} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(titles.length).toBeGreaterThan(0));
  await new Promise((r) => setTimeout(r, 50));
  expect(titles[titles.length - 1]).toBe("Open Pull Requests");
  expect(titles).not.toContain("PRs (lens)");
});
