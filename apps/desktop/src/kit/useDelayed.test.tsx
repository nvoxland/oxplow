import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { DelayedLoading, LOADING_AFTER_MS } from "./useDelayed.js";

afterEach(cleanup);

const wait = (ms: number) => act(() => new Promise<void>((r) => setTimeout(r, ms)));

test("loading text is absent at first and shows only after the delay", async () => {
  const view = render(<DelayedLoading ms={40}><p>Loading…</p></DelayedLoading>);
  expect(view.queryByText("Loading…")).toBeNull();
  await wait(80);
  expect(view.queryByText("Loading…")).not.toBeNull();
});

test("a load that ends before the delay never shows the text", async () => {
  const view = render(<DelayedLoading ms={60}><p>Loading…</p></DelayedLoading>);
  await wait(20);
  view.unmount();
  expect(document.body.textContent).toBe("");
});

test("the default delay is about 300 ms", () => {
  expect(LOADING_AFTER_MS).toBe(300);
});
