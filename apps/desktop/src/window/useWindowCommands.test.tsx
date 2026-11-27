import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { CommandSpec, CommandUi } from "../tauri-bridge/generated/bindings.js";

// The window's commands: what a command's `when` reads about the window,
// a shortcut runs its offer, and a command the window hosts runs in the
// window, not on the daemon.

function spec(id: string, ui: Partial<CommandUi>, op?: { scope: string; op: string }): CommandSpec {
  return {
    id,
    summary: "",
    input_schema: {},
    job: false,
    confirm: "never",
    undoable: false,
    lifecycle: "stable",
    atomicity: "tx",
    access: "write",
    needs: [],
    op: op ?? null,
    ui: { label: id, group: null, keywords: [], about: null, input: null, form: null, open_after: null, background: false, shortcut: null, while_typing: false, menu: null, when: null, ...ui },
  } as unknown as CommandSpec;
}
const specs = [spec("oxplow.window.find", { shortcut: "ctrl/cmd+f", while_typing: true }, { scope: "window.show", op: "find" })];
mock.module("../personCommandsStore.js", () => ({ usePersonCommands: () => specs }));
const daemonRuns: string[] = [];
const realPerson = { ...(await import("../personCommands.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../personCommands.js", () => ({
  ...realPerson,
  personCommands: { run: async (_label: string, id: string) => (daemonRuns.push(id), null) },
}));
const { useWindowCommands } = await import("./useWindowCommands.js");

afterEach(() => {
  daemonRuns.length = 0;
  cleanup();
});

function setup(finds: string[]) {
  return renderHook(() =>
    useWindowCommands({
      stream: { id: "str1", kind: "worktree" } as never,
      selectedThread: { id: "thr1" } as never,
      workspaceContext: { vcsEnabled: true },
      harnesses: [],
      tabs: { centerActive: "oxplow:oxplow/file?path=a.ts", dirtyTabs: { "oxplow:oxplow/file?path=a.ts": true }, shownFilePath: "a.ts" },
      handleOpenPageRef: { current: null },
      windowHandlersRef: { current: { "window.show": { find: () => (finds.push("found"), null) } } },
      navKeysRef: { current: { back: () => {}, forward: () => {}, step: () => {}, mode: () => {} } },
    }),
  );
}

test("a command's `when` reads the window: a file shown, with unsaved edits", () => {
  const hook = setup([]);
  expect(hook.result.current.whenContext).toMatchObject({ streamShown: true, threadShown: true, fileShown: true, pageDirty: true, vcsEnabled: true });
});

test("a shortcut runs its offer, and one the window hosts runs in the window", async () => {
  const finds: string[] = [];
  setup(finds);
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, metaKey: true }));
    await Promise.resolve();
  });
  expect(finds).toEqual(["found"]);
  expect(daemonRuns).toEqual([]);
});
