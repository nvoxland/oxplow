import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { createFailureStore, createInlineClaims } from "../commands/actionFailures.js";
import { createRunner } from "../commands/runForPerson.js";
import { RunnerContext } from "../commands/RunnerContext.js";

import { formatShortcut } from "../keybindings.js";
import { ContextMenu, MenuList } from "./ContextMenu.js";

afterEach(cleanup);

/** A list scrolls in place (it may be taller than the window), so a submenu
 *  laid out inside it would be clipped by it: it is placed on the window,
 *  beside its item, with its line of intent above its items. */
test("a submenu escapes its list's scroll box, beside its item", () => {
  const { getByTestId } = render(
    <MenuList
      items={[
        {
          id: "mark",
          label: "Mark with…",
          enabled: true,
          submenuHeader: "What it does.",
          submenu: [{ id: "mark.red", label: "Red", enabled: true }],
        },
      ]}
    />,
  );
  fireEvent.click(getByTestId("menu-item-mark"));
  const header = getByTestId("context-submenu-header");
  expect(header.textContent).toBe("What it does.");
  const box = getByTestId("context-submenu");
  expect(box.style.position).toBe("fixed");
  expect(box.contains(getByTestId("menu-item-mark.red"))).toBe(true);
});

/** A mouse reaches a submenu's item before it clicks it: the hover opens
 *  the submenu, and the click keeps it open rather than closing it. */
test("a click after the hover that opened a submenu keeps it open", () => {
  const { getByTestId, queryByTestId } = render(
    <MenuList
      items={[
        { id: "threads", label: "Threads", enabled: true, submenu: [{ id: "threads.hand-off", label: "Hand Off Work…", enabled: true }] },
      ]}
    />,
  );
  const item = getByTestId("menu-item-threads");
  fireEvent.mouseEnter(item.parentElement!);
  expect(queryByTestId("menu-item-threads.hand-off")).not.toBeNull();
  fireEvent.click(item);
  expect(queryByTestId("menu-item-threads.hand-off")).not.toBeNull();
});

test("a context menu has one frame: its list draws no second border inside it", () => {
  render(<ContextMenu items={[{ id: "a", label: "A", run: () => {} }]} position={{ x: 10, y: 10 }} onClose={() => {}} />);
  const framed = [...document.body.querySelectorAll<HTMLElement>("div")].filter((d) => (d.getAttribute("style") ?? "").includes("--border-strong"));
  expect(framed).toHaveLength(1);
});

/** A command's shortcut is declared for every platform (`Ctrl/Cmd+Shift+N`);
 *  the menu shows this platform's keys, as the launcher does. */
test("a menu item shows its shortcut in this platform's keys", () => {
  const { getByTestId } = render(
    <MenuList items={[{ id: "new", label: "New Task…", enabled: true, shortcut: "Ctrl/Cmd+Shift+N" }]} />,
  );
  const text = getByTestId("menu-item-new").textContent ?? "";
  expect(text).not.toContain("Ctrl/Cmd");
  expect(text).toContain(formatShortcut("Ctrl/Cmd+Shift+N"));
});

// A menu item whose run fails is the person's failed command: recorded,
// and (the menu closing) the toast's. The menu still closes.
test("a menu item that fails is recorded, and the menu closes", async () => {
  const reports: string[] = [];
  const runner = createRunner({
    report: async (e) => (reports.push(`${e.label}: ${e.message}`), null),
    failures: createFailureStore(),
    claims: createInlineClaims(),
    now: () => 0,
  });
  let closed = 0;
  const { getByTestId } = render(
    <RunnerContext.Provider value={runner}>
      <MenuList
        items={[{ id: "push", label: "Push", enabled: true, run: async () => Promise.reject(new Error("rejected: non-fast-forward")) }]}
        onAction={() => (closed += 1)}
      />
    </RunnerContext.Provider>,
  );
  fireEvent.click(getByTestId("menu-item-push"));
  await waitFor(() => expect(closed).toBe(1));
  expect(reports).toEqual(["Push: rejected: non-fast-forward"]);
});
