import { expect, test } from "bun:test";

import { IpcCallError } from "./ipc-error.js";
import { createPersonCommands } from "./personCommands.js";
import type { CommandOutcome } from "./tauri-bridge/generated/bindings.js";

// A command the person runs outside a page (a launcher entry): it runs;
// when it asks, it waits for the person, then runs confirmed.
test("a command that asks waits for the person, then runs confirmed; Cancel drops it", async () => {
  const calls: Array<[string, unknown, boolean]> = [];
  const toasts: string[] = [];
  const store = createPersonCommands({
    runCommand: async (name, input, confirmed) => {
      calls.push([name, input, confirmed]);
      if (!confirmed) throw new IpcCallError("asks", "NEEDS_CONFIRMATION");
      return { result: null, audit_id: 1, event_id: null, inverse: null } satisfies CommandOutcome;
    },
    undo: async () => {},
    toast: (m) => toasts.push(m),
    recordError: () => {},
  });
  await store.run("New Bug", "oxplow.work_item.create", { title: "Bug" });
  expect(store.pending()).toEqual({ label: "New Bug", command: "oxplow.work_item.create", input: { title: "Bug" } });
  await store.confirm();
  expect(calls).toEqual([
    ["oxplow.work_item.create", { title: "Bug" }, false],
    ["oxplow.work_item.create", { title: "Bug" }, true],
  ]);
  expect(store.pending()).toBeNull();
  expect(toasts).toEqual(["New Bug: done."]);

  await store.run("New Bug", "oxplow.work_item.create", { title: "Bug" });
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
    undo: async () => {},
    toast: () => {},
    recordError: (label, message) => errors.push(`${label}: ${message}`),
  });
  await store.run("New Bug", "oxplow.work_item.create", {});
  expect(errors).toEqual(["New Bug: bad input"]);
  expect(store.pending()).toBeNull();
});

// tsk975: a command that came back undoable offers Undo on its toast;
// taking it undoes that run, as the person. One that isn't offers none.
test("an undoable command's toast offers Undo, which undoes that run", async () => {
  const toasts: Array<[string, (() => void) | undefined]> = [];
  const undone: number[] = [];
  const store = createPersonCommands({
    runCommand: async (name) =>
      ({
        result: null,
        audit_id: name === "oxplow.work_item.transition" ? 7 : 8,
        event_id: null,
        inverse: name === "oxplow.work_item.transition" ? { name: "oxplow.work_item.transition", input: {} } : null,
      }) satisfies CommandOutcome,
    undo: async (auditId) => {
      undone.push(auditId);
    },
    toast: (m, undo) => toasts.push([m, undo]),
    recordError: () => {},
  });
  await store.run("Move to Done", "oxplow.work_item.transition", { ref: "work_item:oxplow:tsk1", to: "done" });
  await store.run("Comment", "oxplow.work_item.comment", {});
  expect(toasts.map(([m, u]) => [m, u !== undefined])).toEqual([
    ["Move to Done: done.", true],
    ["Comment: done.", false],
  ]);
  toasts[0]![1]!();
  await new Promise((r) => setTimeout(r, 0));
  expect(undone).toEqual([7]);
  expect(toasts.at(-1)![0]).toBe("Move to Done: undone.");
});
