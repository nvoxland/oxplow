import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { TerminalTouchBar } from "./TerminalTouchBar.js";

afterEach(cleanup);

function bar(over: Partial<Parameters<typeof TerminalTouchBar>[0]> = {}) {
  const sent: string[] = [];
  const inserted: string[] = [];
  const ctrl: boolean[] = [];
  let escapes = 0;
  const view = render(
    <TerminalTouchBar
      send={(bytes) => sent.push(bytes)}
      applicationCursor={() => false}
      ctrl={false}
      onCtrl={(on) => ctrl.push(on)}
      onEscape={() => escapes++}
      onInsert={(text) => inserted.push(text)}
      {...over}
    />,
  );
  return { ...view, sent, inserted, ctrl, escapes: () => escapes };
}

test("a key sends its bytes, and Esc also says the person interrupted", () => {
  const b = bar();
  fireEvent.click(b.getByTestId("terminal-key-esc"));
  fireEvent.click(b.getByTestId("terminal-key-tab"));
  fireEvent.click(b.getByTestId("terminal-key-backtab"));
  expect(b.sent).toEqual(["\x1b", "\t", "\x1b[Z"]);
  expect(b.escapes()).toBe(1);
});

test("the arrows ask the terminal for its cursor-key mode at the press", () => {
  let app = false;
  const b = bar({ applicationCursor: () => app });
  fireEvent.click(b.getByTestId("terminal-key-up"));
  app = true;
  fireEvent.click(b.getByTestId("terminal-key-up"));
  expect(b.sent).toEqual(["\x1b[A", "\x1bOA"]);
});

test("Ctrl is a toggle that shows whether it is on", () => {
  const off = bar();
  fireEvent.click(off.getByTestId("terminal-key-ctrl"));
  expect(off.ctrl).toEqual([true]);
  expect(off.getByTestId("terminal-key-ctrl").getAttribute("aria-pressed")).toBe("false");
  cleanup();
  const on = bar({ ctrl: true });
  expect(on.getByTestId("terminal-key-ctrl").getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(on.getByTestId("terminal-key-ctrl"));
  expect(on.ctrl).toEqual([false]);
});

test("pressing a key leaves the focus (and the on-screen keyboard) in the terminal", () => {
  const b = bar();
  // A press that isn't cancelled would move the focus to the button and
  // close the keyboard.
  expect(fireEvent.pointerDown(b.getByTestId("terminal-key-left"))).toBe(false);
  expect(fireEvent.mouseDown(b.getByTestId("terminal-key-left"))).toBe(false);
});

test("Compose is a real text field: Insert puts its text in the terminal, never Enter", () => {
  const b = bar();
  fireEvent.click(b.getByTestId("terminal-compose"));
  const field = b.getByTestId("terminal-compose-text") as HTMLTextAreaElement;
  expect((b.getByTestId("terminal-compose-insert") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(field, { target: { value: "explain the failing test" } });
  fireEvent.click(b.getByTestId("terminal-compose-insert"));
  expect(b.inserted).toEqual(["explain the failing test"]);
  expect(b.sent).toEqual([]);
  expect(b.queryByTestId("terminal-compose-text")).toBeNull();
});

test("in the compose field Enter adds a line and Cmd/Ctrl+Enter inserts", () => {
  const b = bar();
  fireEvent.click(b.getByTestId("terminal-compose"));
  fireEvent.change(b.getByTestId("terminal-compose-text"), { target: { value: "line one\nline two" } });
  fireEvent.keyDown(b.getByTestId("terminal-compose-text"), { key: "Enter" });
  expect(b.inserted).toEqual([]);
  fireEvent.keyDown(b.getByTestId("terminal-compose-text"), { key: "Enter", ctrlKey: true });
  expect(b.inserted).toEqual(["line one\nline two"]);
});

test("Escape in the compose field cancels it, and Cancel does too", () => {
  const b = bar();
  fireEvent.click(b.getByTestId("terminal-compose"));
  fireEvent.change(b.getByTestId("terminal-compose-text"), { target: { value: "draft" } });
  fireEvent.keyDown(b.getByTestId("terminal-compose-text"), { key: "Escape" });
  expect(b.queryByTestId("terminal-compose-text")).toBeNull();
  fireEvent.click(b.getByTestId("terminal-compose"));
  fireEvent.click(b.getByTestId("terminal-compose-cancel"));
  expect(b.queryByTestId("terminal-compose-text")).toBeNull();
  expect(b.inserted).toEqual([]);
});
