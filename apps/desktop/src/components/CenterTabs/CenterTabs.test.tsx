import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { CenterTabs, type CenterTab } from "./CenterTabs.js";

afterEach(cleanup);

function makeTabs(): CenterTab[] {
  return [
    { id: "agent", label: "Agent", closable: false, render: () => null },
    { id: "file:a.ts", label: "a.ts", closable: true, render: () => null },
    { id: "file:b.ts", label: "b.ts", closable: true, render: () => null },
    { id: "diff:c", label: "c", closable: true, render: () => null },
  ];
}

function renderTabs(opts: {
  activeId?: string;
  onClose?: (id: string) => void;
  onActivate?: (id: string) => void;
}) {
  const onClose = opts.onClose ?? (() => {});
  const onActivate = opts.onActivate ?? (() => {});
  return render(
    <CenterTabs
      tabs={makeTabs()}
      activeId={opts.activeId ?? "file:b.ts"}
      onActivate={onActivate}
      onClose={onClose}
      onReorder={() => {}}
    />,
  );
}

test("right-click any tab exposes the universal close items", () => {
  const { getByTestId } = renderTabs({});
  fireEvent.contextMenu(getByTestId("center-tab-file:b.ts"), { clientX: 10, clientY: 10 });
  expect(getByTestId("menu-item-tab.close-others")).toBeTruthy();
  expect(getByTestId("menu-item-tab.close-right")).toBeTruthy();
});

test("Close Other Tabs closes every closable tab except the anchor", () => {
  const closed: string[] = [];
  const { getByTestId } = renderTabs({ onClose: (id) => closed.push(id) });
  fireEvent.contextMenu(getByTestId("center-tab-file:b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-others"));
  expect(closed).toEqual(["file:a.ts", "diff:c"]);
});

test("Close Tabs to the Right closes only tabs after the anchor", () => {
  const closed: string[] = [];
  const { getByTestId } = renderTabs({ onClose: (id) => closed.push(id) });
  fireEvent.contextMenu(getByTestId("center-tab-file:a.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-right"));
  expect(closed).toEqual(["file:b.ts", "diff:c"]);
});

test("Close Tabs to the Right is disabled on the last tab", () => {
  const { getByTestId } = renderTabs({});
  fireEvent.contextMenu(getByTestId("center-tab-diff:c"), { clientX: 10, clientY: 10 });
  expect((getByTestId("menu-item-tab.close-right") as HTMLButtonElement).disabled).toBe(true);
});

test("closing a batch that swept the active tab refocuses the anchor", () => {
  const closed: string[] = [];
  const activated: string[] = [];
  // Active is file:a.ts; right-click file:b.ts and Close Other Tabs (closes a + c).
  const { getByTestId } = renderTabs({
    activeId: "file:a.ts",
    onClose: (id) => closed.push(id),
    onActivate: (id) => activated.push(id),
  });
  fireEvent.contextMenu(getByTestId("center-tab-file:b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-others"));
  expect(closed).toEqual(["file:a.ts", "diff:c"]);
  // The active tab (a.ts) was closed → selection falls back to the anchor.
  expect(activated).toEqual(["file:b.ts"]);
});

test("closing tabs that leave the active tab open does not refocus", () => {
  const activated: string[] = [];
  // Active is file:a.ts (left of anchor); Close Tabs to the Right of b closes
  // only c, so a stays active and we must not steal focus.
  const { getByTestId } = renderTabs({
    activeId: "file:a.ts",
    onActivate: (id) => activated.push(id),
  });
  fireEvent.contextMenu(getByTestId("center-tab-file:b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-right"));
  expect(activated).toEqual([]);
});
