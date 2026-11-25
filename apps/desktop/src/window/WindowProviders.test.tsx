import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";
import { usePanelLayout } from "../components/Panels/PanelLayoutContext.js";

// What every shell's window sits in: the panels' runs and layout are there
// for whatever the shell draws inside.

const { WindowProviders } = await import("./WindowProviders.js");

afterEach(() => cleanup());

function ReadsLayout() {
  usePanelLayout();
  return <div data-testid="inside">inside</div>;
}

test("a shell's parts are drawn inside the window's panel providers", () => {
  const view = render(
    <WindowProviders streamId={null} threadId={null} session={undefined} onLiveItems={() => {}} onReviewAlerts={() => {}}>
      <ReadsLayout />
    </WindowProviders>,
  );
  expect(view.getByTestId("inside").textContent).toBe("inside");
});
