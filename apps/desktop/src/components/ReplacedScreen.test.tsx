import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

// Another page load of this device took over: this window says so, over
// everything, and its one action takes the device back by loading again.

const realTransport = { ...(await import("../tauri-bridge/transport.js")) }; // a copy: `mock.module` patches a loaded module in place
let tell: ((state: string) => void) | null = null;
let reloads = 0;
mock.module("../tauri-bridge/transport.js", () => ({
  ...realTransport,
  isReplaced: () => false,
  onRemoteConnectionState: (handler: (state: string) => void) => {
    tell = handler;
    return () => {
      tell = null;
    };
  },
}));
const { ReplacedScreen } = await import("./ReplacedScreen.js");

afterEach(() => cleanup());

test("the window shows nothing until it is replaced, then says so and can take the device back", () => {
  const view = render(<ReplacedScreen reload={() => (reloads += 1)} />);
  expect(view.queryByTestId("replaced-screen")).toBeNull();
  act(() => tell!("down"));
  expect(view.queryByTestId("replaced-screen")).toBeNull();
  act(() => tell!("replaced"));
  expect(view.getByTestId("replaced-screen").textContent).toContain("open in another tab");
  act(() => view.getByTestId("replaced-use-here").click());
  expect(reloads).toBe(1);
});
