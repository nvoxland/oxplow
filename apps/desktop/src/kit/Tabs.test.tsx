import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { Tabs } from "./Tabs.js";

afterEach(cleanup);

const tabs = [
  { id: "a", title: "A" },
  { id: "b", title: "B" },
  { id: "c", title: "C" },
];

test("Home and End go to the first and last tab; a modified arrow is left to the shell", () => {
  const picked: string[] = [];
  const { getAllByRole } = render(
    <Tabs label="Letters" tabs={tabs} current="b" onPick={(id) => picked.push(id)}>
      <p>B's panel</p>
    </Tabs>,
  );
  const b = getAllByRole("tab")[1]!;
  fireEvent.keyDown(b, { key: "Home" });
  fireEvent.keyDown(b, { key: "End" });
  expect(picked).toEqual(["a", "c"]);
  // Alt/Cmd+arrows are the shell's (history, words): no tab changes.
  fireEvent.keyDown(b, { key: "ArrowLeft", altKey: true });
  fireEvent.keyDown(b, { key: "ArrowRight", metaKey: true });
  expect(picked).toEqual(["a", "c"]);
});

test("tabs that fill pass the height down to the panel", () => {
  const { container } = render(
    <Tabs label="Letters" tabs={tabs} current="a" fill>
      <p>A</p>
    </Tabs>,
  );
  const root = container.querySelector(".ox-tabs") as HTMLElement;
  const panel = container.querySelector('[role="tabpanel"]') as HTMLElement;
  expect(root.style.flex).toContain("1");
  expect(panel.style.flex).toContain("1");
  expect(["0", "0px"]).toContain(panel.style.minHeight);
});
