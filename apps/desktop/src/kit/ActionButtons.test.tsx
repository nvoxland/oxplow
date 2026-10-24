import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { ContextMenu } from "../components/ContextMenu.js";
import { ActionButtons } from "./ActionButtons.js";

afterEach(cleanup);

test("an action with an icon is the icon alone, its label the hover and its name", () => {
  const ran: string[] = [];
  const view = render(
    <ActionButtons
      actions={[
        { id: "fit", label: "Fit", icon: "maximize-2", title: "Show every item", run: () => void ran.push("fit") },
        { id: "today", label: "Today", run: () => void ran.push("today") },
      ]}
    />,
  );
  const fit = view.getByTestId("action-fit");
  expect(fit.textContent).toBe("");
  expect(fit.querySelector("svg")).not.toBeNull();
  expect(fit.getAttribute("title")).toBe("Fit: Show every item");
  expect(fit.getAttribute("aria-label")).toBe("Fit");
  fireEvent.click(fit);
  // One without an icon shows its label.
  expect(view.getByTestId("action-today").textContent).toBe("Today");
  expect(ran).toEqual(["fit"]);
});

test("a menu item with an icon draws it beside its label", () => {
  const view = render(
    <ContextMenu
      items={[
        { id: "copy", label: "Copy", enabled: true, icon: "copy" },
        { id: "plain", label: "Plain", enabled: true },
      ]}
      position={{ x: 0, y: 0 }}
      onClose={() => {}}
    />,
  );
  const copy = view.getByTestId("menu-item-copy");
  expect(copy.textContent).toContain("Copy");
  expect(copy.querySelector('[data-testid="menu-item-icon"] svg')).not.toBeNull();
  expect(view.getByTestId("menu-item-plain").querySelector('[data-testid="menu-item-icon"]')).toBeNull();
});
