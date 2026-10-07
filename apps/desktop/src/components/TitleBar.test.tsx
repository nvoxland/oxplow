import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Stream } from "../api.js";
import { subscribeNavigatorOpenRequests } from "../navigator-bus.js";
import { SEARCH_TRIGGER_TESTID, TitleBar } from "./TitleBar.js";

afterEach(cleanup);

const stream = { id: "str2", title: "Bugfixes", branch: "fix/scan" } as unknown as Stream;

function renderBar(over: Partial<Parameters<typeof TitleBar>[0]> = {}) {
  const calls = { search: 0, threadSettings: [] as string[] };
  const view = render(
    <TitleBar
      stream={stream}
      thread={{ id: "thr5", title: "Thread" }}
      vcsEnabled
      leftInset={78}
      onOpenSearch={() => calls.search++}
      onOpenThreadSettings={(id) => calls.threadSettings.push(id)}
      {...over}
    />,
  );
  return { view, calls };
}

test("it names the stream and thread at the start, and the branch just before search", () => {
  const { view } = renderBar();
  expect(view.getByTestId("title-bar-stream").textContent).toBe("Bugfixes");
  expect(view.getByTestId("title-bar-thread").textContent).toBe("Thread");
  const columns = [...view.getByTestId("title-bar").children];
  expect(columns.length).toBe(3);
  expect(columns[0].textContent).not.toContain("fix/scan");
  expect(columns[1].textContent).toContain("fix/scan");
  expect(columns[2].getAttribute("data-testid")).toBe(SEARCH_TRIGGER_TESTID);
});

test("the stream name opens the navigator; the thread name its settings", () => {
  let opened = 0;
  const off = subscribeNavigatorOpenRequests(() => opened++);
  const { view, calls } = renderBar();
  fireEvent.click(view.getByTestId("title-bar-stream"));
  expect(opened).toBe(1);
  fireEvent.click(view.getByTestId("title-bar-thread"));
  expect(calls.threadSettings).toEqual(["thr5"]);
  off();
});

test("the search field opens search", () => {
  const { view, calls } = renderBar();
  fireEvent.click(view.getByTestId(SEARCH_TRIGGER_TESTID));
  expect(calls.search).toBe(1);
});

test("its empty space drags the window; its controls don't", () => {
  const { view } = renderBar();
  expect(view.getByTestId("title-bar").hasAttribute("data-tauri-drag-region")).toBe(true);
  expect(view.getByTestId("title-bar-stream").hasAttribute("data-tauri-drag-region")).toBe(false);
  expect(view.getByTestId(SEARCH_TRIGGER_TESTID).hasAttribute("data-tauri-drag-region")).toBe(false);
});

test("with no thread selected it shows the stream alone", () => {
  const { view } = renderBar({ thread: null });
  expect(view.getByTestId("title-bar-stream").textContent).toBe("Bugfixes");
  expect(view.queryByTestId("title-bar-thread")).toBeNull();
});
