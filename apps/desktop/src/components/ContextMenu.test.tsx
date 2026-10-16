import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { MenuList } from "./ContextMenu.js";

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
