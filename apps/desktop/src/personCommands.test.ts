import { expect, test } from "bun:test";

import { IpcCallError } from "./ipc-error.js";
import { createPersonCommands } from "./personCommands.js";

// A command the person runs outside a page (a launcher entry): it runs;
// when it asks, it waits for the person, then runs confirmed.
test("a command that asks waits for the person, then runs confirmed; Cancel drops it", async () => {
  const calls: Array<[string, unknown, boolean]> = [];
  const toasts: string[] = [];
  const store = createPersonCommands({
    runCommand: async (name, input, confirmed) => {
      calls.push([name, input, confirmed]);
      if (!confirmed) throw new IpcCallError("asks", "NEEDS_CONFIRMATION");
      return { result: null, audit_id: 1, event_id: null, undo: null } as never;
    },
    toast: (m) => toasts.push(m),
    recordError: () => {},
  });
  await store.run("New Bug", "work_item.create", { title: "Bug" });
  expect(store.pending()).toEqual({ label: "New Bug", command: "work_item.create", input: { title: "Bug" } });
  await store.confirm();
  expect(calls).toEqual([
    ["work_item.create", { title: "Bug" }, false],
    ["work_item.create", { title: "Bug" }, true],
  ]);
  expect(store.pending()).toBeNull();
  expect(toasts).toEqual(["New Bug: done."]);

  await store.run("New Bug", "work_item.create", { title: "Bug" });
  store.cancel();
  expect(store.pending()).toBeNull();
  expect(calls.length).toBe(3);
});

test("a failure is recorded, not thrown", async () => {
  const errors: string[] = [];
  const store = createPersonCommands({
    runCommand: async () => {
      throw new IpcCallError("bad input", "INVALID");
    },
    toast: () => {},
    recordError: (label, message) => errors.push(`${label}: ${message}`),
  });
  await store.run("New Bug", "work_item.create", {});
  expect(errors).toEqual(["New Bug: bad input"]);
  expect(store.pending()).toBeNull();
});
