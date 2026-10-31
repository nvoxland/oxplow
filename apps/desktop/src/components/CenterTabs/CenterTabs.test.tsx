import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";

import { CenterTabs, type CenterTab } from "./CenterTabs.js";

afterEach(cleanup);

function makeTabs(): CenterTab[] {
  return [
    { id: "oxplow:oxplow/page?page=agent", label: "Agent", closable: false, render: () => null },
    { id: "oxplow:oxplow/file?path=a.ts", label: "a.ts", closable: true, render: () => null },
    { id: "oxplow:oxplow/file?path=b.ts", label: "b.ts", closable: true, render: () => null },
    { id: "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk", label: "c", closable: true, render: () => null },
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
      activeId={opts.activeId ?? "oxplow:oxplow/file?path=b.ts"}
      onActivate={onActivate}
      onClose={onClose}
      onReorder={() => {}}
    />,
  );
}

test("right-click any tab exposes the universal close items", () => {
  const { getByTestId } = renderTabs({});
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=b.ts"), { clientX: 10, clientY: 10 });
  expect(getByTestId("menu-item-tab.close-others")).toBeTruthy();
  expect(getByTestId("menu-item-tab.close-right")).toBeTruthy();
});

test("right-click a closable tab offers Close for that tab", () => {
  const closed: string[] = [];
  const { getByTestId } = renderTabs({ onClose: (id) => closed.push(id) });
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close"));
  expect(closed).toEqual(["oxplow:oxplow/file?path=b.ts"]);
  cleanup();
  const again = renderTabs({});
  fireEvent.contextMenu(again.getByTestId("center-tab-oxplow:oxplow/page?page=agent"), { clientX: 10, clientY: 10 });
  expect(again.queryByTestId("menu-item-tab.close")).toBeNull();
});

test("Close Other Tabs closes every closable tab except the anchor", () => {
  const closed: string[] = [];
  const { getByTestId } = renderTabs({ onClose: (id) => closed.push(id) });
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-others"));
  expect(closed).toEqual(["oxplow:oxplow/file?path=a.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
});

test("Close Tabs to the Right closes only tabs after the anchor", () => {
  const closed: string[] = [];
  const { getByTestId } = renderTabs({ onClose: (id) => closed.push(id) });
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=a.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-right"));
  expect(closed).toEqual(["oxplow:oxplow/file?path=b.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
});

test("Close Tabs to the Right is disabled on the last tab", () => {
  const { getByTestId } = renderTabs({});
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"), { clientX: 10, clientY: 10 });
  expect((getByTestId("menu-item-tab.close-right") as HTMLButtonElement).disabled).toBe(true);
});

test("closing a batch that swept the active tab refocuses the anchor", () => {
  const closed: string[] = [];
  const activated: string[] = [];
  // Active is oxplow:oxplow/file?path=a.ts; right-click oxplow:oxplow/file?path=b.ts and Close Other Tabs (closes a + c).
  const { getByTestId } = renderTabs({
    activeId: "oxplow:oxplow/file?path=a.ts",
    onClose: (id) => closed.push(id),
    onActivate: (id) => activated.push(id),
  });
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-others"));
  expect(closed).toEqual(["oxplow:oxplow/file?path=a.ts", "oxplow:oxplow/page?left=disk&page=diff&path=c&right=disk"]);
  // The active tab (a.ts) was closed → selection falls back to the anchor.
  expect(activated).toEqual(["oxplow:oxplow/file?path=b.ts"]);
});

test("closing tabs that leave the active tab open does not refocus", () => {
  const activated: string[] = [];
  // Active is oxplow:oxplow/file?path=a.ts (left of anchor); Close Tabs to the Right of b closes
  // only c, so a stays active and we must not steal focus.
  const { getByTestId } = renderTabs({
    activeId: "oxplow:oxplow/file?path=a.ts",
    onActivate: (id) => activated.push(id),
  });
  fireEvent.contextMenu(getByTestId("center-tab-oxplow:oxplow/file?path=b.ts"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-tab.close-right"));
  expect(activated).toEqual([]);
});

/// A session's tab closes on its × at once: the close is undone from its
/// toast, not asked about first.
test("a session's tab closes on one click of its ×", () => {
  const closed: string[] = [];
  const tabs: CenterTab[] = [
    { id: "oxplow:oxplow/agent_session?id=ses7", label: "Claude", closable: true, pinned: true, render: () => null },
    { id: "oxplow:oxplow/file?path=a.ts", label: "a.ts", closable: true, render: () => null },
  ];
  const { getByTestId, queryByTestId } = render(
    <CenterTabs
      tabs={tabs}
      activeId="oxplow:oxplow/agent_session?id=ses7"
      onActivate={() => {}}
      onClose={(id) => closed.push(id)}
      onReorder={() => {}}
    />,
  );
  fireEvent.click(getByTestId("center-tab-close-oxplow:oxplow/agent_session?id=ses7"));
  expect(closed).toEqual(["oxplow:oxplow/agent_session?id=ses7"]);
  expect(queryByTestId("center-tab-close-oxplow:oxplow/agent_session?id=ses7-confirm")).toBeNull();
});

// An agent session's tab shows its harness's logo, else the generic robot.
test("an agent session's tab draws its harness's declared logo", () => {
  const mark = "data:image/svg+xml;base64,PHN2Zy8+";
  const tabs: CenterTab[] = [
    { id: "oxplow:oxplow/agent_session?id=ses1", label: "Claude", closable: true, icon: mark, render: () => null },
    { id: "oxplow:oxplow/agent_session?id=ses2", label: "Other", closable: true, render: () => null },
    { id: "oxplow:oxplow/file?path=a.ts", label: "a.ts", closable: true, icon: mark, render: () => null },
  ];
  const { getByTestId } = render(
    <CenterTabs tabs={tabs} activeId="oxplow:oxplow/file?path=a.ts" onActivate={() => {}} onClose={() => {}} onReorder={() => {}} />,
  );
  const within = (id: string) => getByTestId(`center-tab-${id}`).querySelector('[data-testid="agent-icon"]');
  expect(within("oxplow:oxplow/agent_session?id=ses1")?.getAttribute("data-icon")).toBe("declared");
  expect(within("oxplow:oxplow/agent_session?id=ses2")?.getAttribute("data-icon")).toBe("fallback");
  // Only a session's tab: another kind keeps its own kind icon.
  expect(within("oxplow:oxplow/file?path=a.ts")).toBeNull();
});

// A page whose params changed is the same page: with the same `key` its
// body stays mounted while its id changes, so its state survives.
test("a body with the same key keeps its state when its id changes", () => {
  function Counter() {
    const [n, setN] = useState(0);
    return (
      <button type="button" data-testid="count" onClick={() => setN(n + 1)}>
        {n}
      </button>
    );
  }
  const tab = (id: string): CenterTab => ({ id, key: "t1/oxplow:oxplow/page?page=x/y", label: "Page", closable: true, render: () => <Counter /> });
  const view = render(<CenterTabs tabs={[tab("oxplow:oxplow/page?page=x/y")]} activeId="oxplow:oxplow/page?page=x/y" onActivate={() => {}} onClose={() => {}} onReorder={() => {}} />);
  fireEvent.click(view.getByTestId("count"));
  view.rerender(<CenterTabs tabs={[tab("oxplow:oxplow/page?page=x/y&range=7d")]} activeId="oxplow:oxplow/page?page=x/y&range=7d" onActivate={() => {}} onClose={() => {}} onReorder={() => {}} />);
  expect(view.getByTestId("count").textContent).toBe("1");
  expect(view.getByTestId("center-tab-body-oxplow:oxplow/page?page=x/y&range=7d")).not.toBeNull();
});

test("a tab whose page holds unsaved edits says ● before its label", () => {
  const tabs = makeTabs().map((t) => (t.label === "a.ts" ? { ...t, dirty: true } : t));
  const { getByTestId, queryByTestId } = render(
    <CenterTabs tabs={tabs} activeId={tabs[1]!.id} onActivate={() => {}} onClose={() => {}} onReorder={() => {}} />,
  );
  expect(getByTestId(`center-tab-${tabs[1]!.id}`).textContent).toContain("● a.ts");
  expect(queryByTestId(`center-tab-dirty-${tabs[2]!.id}`)).toBeNull();
});

// A body that moves in the DOM reloads any frame in it (a custom
// component's iframe loads again and reads as navigating away). The strip
// may reorder; the bodies never move.
test("reordering the tabs moves no tab body", () => {
  const tab = (id: string): CenterTab => ({ id, label: id, closable: true, render: () => <div>{id}</div> });
  const props = { onActivate: () => {}, onClose: () => {}, onReorder: () => {} };
  const view = render(<CenterTabs tabs={[tab("a"), tab("b"), tab("c")]} activeId="a" {...props} />);
  const parent = view.getByTestId("center-tab-body-a").parentElement!;
  const moved: string[] = [];
  const observer = new MutationObserver((records) => {
    for (const r of records) for (const n of Array.from(r.removedNodes)) moved.push((n as HTMLElement).dataset?.testid ?? "?");
  });
  observer.observe(parent, { childList: true });
  view.rerender(<CenterTabs tabs={[tab("c"), tab("a"), tab("d"), tab("b")]} activeId="c" {...props} />);
  // Records are delivered on the next microtask; takeRecords reads them now.
  for (const r of observer.takeRecords()) for (const n of Array.from(r.removedNodes)) moved.push((n as HTMLElement).dataset?.testid ?? "?");
  observer.disconnect();
  expect(moved).toEqual([]);
});
