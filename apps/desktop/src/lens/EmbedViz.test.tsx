import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// The Web Page tab: the page opens in a sandboxed window once per entry; the
// panel offers Open again, Open in Browser and Copy URL; a URL the allowlist
// refuses opens nothing.
const realApi = await import("../api.js");
const pristineApi = { ...realApi };
const opened: string[] = [];
const browser: string[] = [];
let answer: { ok: boolean; reason?: string } = { ok: true };
mock.module("../api.js", () => ({
  ...realApi,
  desktopBridge: () => ({
    openExternalUrl: async (url: string) => {
      opened.push(url);
      return answer;
    },
  }),
  openInSystemBrowser: async (url: string) => void browser.push(url),
}));
const { EmbedViz } = await import("./EmbedViz.js");

afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});
afterEach(() => {
  cleanup();
  opened.length = 0;
  browser.length = 0;
  answer = { ok: true };
});

/** A tab entry whose history state starts as `state` and keeps what's written. */
function inEntry(state: Record<string, unknown>, ui: React.ReactNode) {
  const entry: Record<string, unknown> = { ...state };
  const nav = {
    historyState: entry,
    setHistoryState: (patch: Record<string, unknown>) => Object.assign(entry, patch),
  } as unknown as PageNavigation;
  return render(<PageNavigationContext.Provider value={nav}>{ui}</PageNavigationContext.Provider>);
}

test("an allowed page opens once, in the sandboxed window", async () => {
  const view = inEntry({}, <EmbedViz url="https://example.com/a" />);
  await waitFor(() => expect(opened).toEqual(["https://example.com/a"]));
  expect(view.getByTestId("lens-embed").textContent).toContain("example.com");
});

test("a restored entry that already opened it doesn't open it again; Open again does", async () => {
  const view = inEntry({ opened: "https://example.com/a" }, <EmbedViz url="https://example.com/a" />);
  await new Promise((r) => setTimeout(r, 30));
  expect(opened).toEqual([]);
  fireEvent.click(view.getByTestId("lens-embed-reopen"));
  await waitFor(() => expect(opened).toEqual(["https://example.com/a"]));
});

test("a refusal from the window is shown on the panel", async () => {
  answer = { ok: false, reason: "no window" };
  const view = inEntry({}, <EmbedViz url="https://example.com/" />);
  expect((await view.findByTestId("lens-embed-error")).textContent).toBe("no window");
});

test("a URL the allowlist refuses draws Couldn't open link and opens nothing", async () => {
  const view = inEntry({}, <EmbedViz url="javascript:alert(1)" />);
  expect(view.getByTestId("lens-embed-blocked").textContent).toContain("Couldn't open link");
  await new Promise((r) => setTimeout(r, 30));
  expect(opened).toEqual([]);
});

test("Open in Browser hands the URL to the system browser", async () => {
  const view = inEntry({ opened: "https://example.com/a" }, <EmbedViz url="https://example.com/a" />);
  fireEvent.click(view.getByTestId("lens-embed-browser"));
  await waitFor(() => expect(browser).toEqual(["https://example.com/a"]));
});

test("a page that names no URL says so", () => {
  const view = inEntry({}, <EmbedViz url={null} />);
  expect(view.container.textContent).toContain("names no web page");
});
