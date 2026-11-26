import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { MoreButton } from "./MoreButton.js";

const real = globalThis.matchMedia;
function pointer(coarse: boolean) {
  globalThis.matchMedia = ((query: string) =>
    ({ matches: coarse && query.includes("coarse"), addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList) as typeof matchMedia;
}
afterEach(() => {
  cleanup();
  globalThis.matchMedia = real;
});

// A mouse has the right-click: no button.
test("a mouse sees no More button", () => {
  pointer(false);
  const view = render(<MoreButton testId="more" onOpen={() => {}} />);
  expect(view.queryByTestId("more")).toBeNull();
});

// A finger sees it; a tap opens the menu below it and is not its row's.
test("a finger's tap opens the menu below the button, not the row", () => {
  pointer(true);
  const at: Array<{ x: number; y: number }> = [];
  let row = 0;
  const view = render(
    <div onClick={() => row++}>
      <MoreButton testId="more" label="Actions for One" onOpen={(p) => at.push(p)} />
    </div>,
  );
  const button = view.getByTestId("more");
  expect(button.getAttribute("aria-label")).toBe("Actions for One");
  fireEvent.click(button);
  expect(at.length).toBe(1);
  expect(row).toBe(0);
});
