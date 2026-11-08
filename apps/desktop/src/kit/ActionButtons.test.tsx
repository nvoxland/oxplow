import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import { ContextMenu } from "../components/ContextMenu.js";
import { ActionButtons } from "./ActionButtons.js";
import { createFailureStore, createInlineClaims } from "../commands/actionFailures.js";
import { createRunner } from "../commands/runForPerson.js";
import { RunnerContext } from "../commands/RunnerContext.js";

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

// A failed action says so beside its button, with the way to its record;
// it is recorded once, and running it again clears it.
test("a failed action shows its failure beside its button, recorded once", async () => {
  const reports: string[] = [];
  const runner = createRunner({
    report: async (e) => (reports.push(e.label), "ev1"),
    failures: createFailureStore(),
    claims: createInlineClaims(),
    now: () => 1,
  });
  let fail = true;
  const view = render(
    <RunnerContext.Provider value={runner}>
      <ActionButtons
        actions={[
          {
            id: "deploy",
            label: "Deploy",
            run: async () => {
              if (fail) throw new Error("the schema changed since it was recorded\nsecond line");
            },
          },
        ]}
      />
    </RunnerContext.Provider>,
  );
  fireEvent.click(view.getByTestId("action-deploy"));
  const shown = await waitFor(() => view.getByTestId("action-error-deploy"));
  // The first line; the whole message is its hover.
  expect(shown.textContent).toContain("the schema changed since it was recorded");
  expect(shown.textContent).not.toContain("second line");
  expect(within(shown).getByTestId("inline-error-details").textContent).toBe("Details");
  expect(reports).toEqual(["Deploy"]);
  fail = false;
  fireEvent.click(view.getByTestId("action-deploy"));
  await waitFor(() => expect(view.queryByTestId("action-error-deploy")).toBeNull());
  expect(reports).toEqual(["Deploy"]);
});
