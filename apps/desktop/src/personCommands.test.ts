import { expect, test } from "bun:test";

import { createFailureStore, createInlineClaims } from "./commands/actionFailures.js";
import { createRunner } from "./commands/runForPerson.js";
import { IpcCallError } from "./ipc-error.js";
import { createPersonCommands } from "./personCommands.js";
import type { CommandOutcome } from "./tauri-bridge/generated/bindings.js";

/** A runner whose record is `errors` (`<label>: <message>`). */
const recording = (errors: string[] = []) =>
  createRunner({
    report: async (e) => (errors.push(`${e.label}: ${e.message}`), null),
    failures: createFailureStore(),
    claims: createInlineClaims(),
    now: () => 0,
  }).run;

// A command the person runs outside a page (a launcher entry): it runs;
// when it asks, it waits for the person, then runs confirmed.
test("a command that asks waits for the person, then runs confirmed; Cancel drops it", async () => {
  const calls: Array<[string, unknown, boolean]> = [];
  const toasts: string[] = [];
  const store = createPersonCommands({
    runCommand: async (name, input, confirmed) => {
      calls.push([name, input, confirmed]);
      if (!confirmed) throw new IpcCallError("asks", "NEEDS_CONFIRMATION");
      return { result: null, command_run_id: 1, event_id: null, inverse: null } satisfies CommandOutcome;
    },
    undo: async () => {},
    toast: (m) => toasts.push(m),
    run: recording(),
  });
  await store.run("New Bug", "acme.issue.create", { title: "Bug" });
  expect(store.pending()).toEqual({ label: "New Bug", command: "acme.issue.create", input: { title: "Bug" }, details: [] });
  await store.confirm();
  expect(calls).toEqual([
    ["acme.issue.create", { title: "Bug" }, false],
    ["acme.issue.create", { title: "Bug" }, true],
  ]);
  expect(store.pending()).toBeNull();
  expect(toasts).toEqual(["✓ New Bug"]);

  await store.run("New Bug", "acme.issue.create", { title: "Bug" });
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
    run: recording(errors),
  });
  await store.run("New Bug", "acme.issue.create", {});
  expect(errors).toEqual(["New Bug: bad input"]);
  expect(store.pending()).toBeNull();
});

// a command that came back undoable offers Undo on its toast;
// taking it undoes that run, as the person. One that isn't offers none.
test("an undoable command's toast offers Undo, which undoes that run", async () => {
  const toasts: Array<[string, (() => void) | undefined]> = [];
  const undone: number[] = [];
  const store = createPersonCommands({
    runCommand: async (name) =>
      ({
        result: null,
        command_run_id: name === "acme.issue.transition" ? 7 : 8,
        event_id: null,
        inverse: name === "acme.issue.transition" ? { name: "acme.issue.transition", input: {} } : null,
      }) satisfies CommandOutcome,
    undo: async (commandRunId) => {
      undone.push(commandRunId);
    },
    toast: (m, undo) => toasts.push([m, undo]),
    run: recording(),
  });
  await store.run("Move to Done", "acme.issue.transition", { uri: "oxplow:oxplow/task?id=tsk1&provider=oxtask", to: "done" });
  await store.run("Comment", "acme.issue.comment", {});
  expect(toasts.map(([m, u]) => [m, u !== undefined])).toEqual([
    ["✓ Move to Done", true],
    ["✓ Comment", false],
  ]);
  toasts[0]![1]!();
  await new Promise((r) => setTimeout(r, 0));
  expect(undone).toEqual([7]);
  expect(toasts.at(-1)![0]).toBe("Undone: Move to Done");
});

test("a result that names a thing gets an Open link on its toast", async () => {
  const opened: string[] = [];
  const links: Array<(() => void) | undefined> = [];
  const store = createPersonCommands({
    runCommand: async (name) =>
      ({ result: name === "a" ? { uri: "task:t1" } : {}, command_run_id: null, event_id: null, inverse: null }) satisfies CommandOutcome,
    undo: async () => {},
    toast: (_m, _undo, open) => links.push(open),
    openPage: (uri) => opened.push(uri),
    run: recording(),
  });
  await store.run("New", "a", {});
  await store.run("Other", "b", {});
  expect(links[1]).toBeUndefined();
  links[0]!();
  expect(opened).toEqual(["task:t1"]);
});
