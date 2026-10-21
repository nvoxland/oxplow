import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { useMonacoViewState, type ViewStateEditor } from "./useMonacoViewState.js";

afterEach(cleanup);

function fakeEditor(state: unknown) {
  const handlers: Array<() => void> = [];
  const restored: unknown[] = [];
  const editor: ViewStateEditor = {
    saveViewState: () => state,
    restoreViewState: (s) => restored.push(s),
    onDidChangeCursorPosition: (cb) => (handlers.push(cb), { dispose() {} }),
    onDidScrollChange: (cb) => (handlers.push(cb), { dispose() {} }),
  };
  return { editor, handlers, restored };
}

let attach: ReturnType<typeof useMonacoViewState>;
function Probe() {
  attach = useMonacoViewState("viewState:a.ts", 0);
  return null;
}
function mount(historyState: Record<string, unknown>, states: Record<string, unknown>[]) {
  const nav = { viewParams: {}, patchViewParams() {}, historyState, setHistoryState: (p: Record<string, unknown>) => states.push(p) } as unknown as PageNavigation;
  render(
    <PageNavigationContext.Provider value={nav}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
}

// Monaco's cursor and scroll belong to the step, as the editor's do: Back
// and a restart land on the same place.
test("attaching restores the step's saved view state and writes the next one back once the editor settles", async () => {
  const states: Record<string, unknown>[] = [];
  mount({ "viewState:a.ts": { top: 40 } }, states);
  const { editor, handlers, restored } = fakeEditor({ top: 90 });
  attach(editor);
  expect(restored).toEqual([{ top: 40 }]);
  handlers.forEach((h) => h());
  handlers.forEach((h) => h());
  await new Promise((r) => setTimeout(r, 5));
  expect(states).toEqual([{ "viewState:a.ts": { top: 90 } }]);
});

test("detaching flushes a pending save", () => {
  const states: Record<string, unknown>[] = [];
  mount({}, states);
  const { editor, handlers } = fakeEditor({ top: 7 });
  const detach = attach(editor);
  handlers[0]();
  detach();
  expect(states).toEqual([{ "viewState:a.ts": { top: 7 } }]);
});
