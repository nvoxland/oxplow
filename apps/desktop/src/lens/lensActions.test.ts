import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { FormStart, LensAction, LensRun } from "../tauri-bridge/generated/bindings.js";
import { actionOffered, offeredWholeLens, copyLens, openAfterAction, performLensAction, promptField, rowRecord, type LensActionDeps } from "./lensActions.js";

const run = { lens: { id: "acme/prs" }, params: { repo: "x" } } as unknown as LensRun;
const action: LensAction = { id: "finish", label: "Finish", command: "oxplow.work_item.transition", input: {}, row: false };

function deps(fail?: Error, openPage: string | null = null) {
  const log: string[] = [];
  const d: LensActionDeps = {
    runLensAction: async (id, a, params, row, streamId, confirmed) => {
      log.push(`run ${id} ${a} ${JSON.stringify(params)} ${JSON.stringify(row)} ${streamId} ${confirmed}`);
      if (fail && !(confirmed && fail instanceof IpcCallError && fail.code === "NEEDS_CONFIRMATION")) throw fail;
      return { result: {}, audit_id: 1, event_id: null, inverse: null };
    },
    toast: (m) => log.push(`toast ${m}`),
    recordError: (label, m) => log.push(`error ${label}: ${m}`),
    openAfter: async (command, result) => {
      log.push(`open_after ${command} ${JSON.stringify(result)}`);
      return openPage;
    },
    openPage: (page, replace) => log.push(`${replace ? "replace with" : "open"} ${page}`),
  };
  return { d, log };
}

test("an action runs its command with the run's params and row", async () => {
  const { d, log } = deps();
  expect(await performLensAction(action, run, { id: 7 }, "str1", false, d)).toBe("done");
  expect(log).toEqual([
    `run acme/prs finish {"repo":"x"} {"id":7} str1 false`,
    "toast Finish: done.",
    "open_after oxplow.work_item.transition {}",
  ]);
});

test("an action whose command names a page to open opens it once it ran", async () => {
  const { d, log } = deps(undefined, "oxplow:oxplow/wiki?slug=lib");
  expect(await performLensAction(action, run, null, null, false, d)).toBe("done");
  expect(log.slice(-2)).toEqual([`open_after oxplow.work_item.transition {}`, "open oxplow:oxplow/wiki?slug=lib"]);
});

const done = { result: {}, audit_id: 1, event_id: null, inverse: null };

test("`then: replace` opens the page in place of the lens's page", async () => {
  const { d, log } = deps(undefined, "oxplow:oxplow/work_item?id=tsk9&provider=oxplow");
  await openAfterAction({ ...action, then: "replace" }, done, d);
  expect(log.at(-1)).toBe("replace with oxplow:oxplow/work_item?id=tsk9&provider=oxplow");
});

test("`then: stay` opens nothing, without asking where", async () => {
  const { d, log } = deps(undefined, "oxplow:oxplow/work_item?id=tsk9&provider=oxplow");
  await openAfterAction({ ...action, then: "stay" }, done, d);
  expect(log).toEqual([]);
});

test("`then: close` closes the lens's page, without asking where", async () => {
  const { d, log } = deps(undefined, "oxplow:oxplow/work_item?id=tsk9&provider=oxplow");
  const closed: string[] = [];
  await openAfterAction({ ...action, then: "close" }, done, { ...d, closePage: () => closed.push("closed") });
  expect(closed).toEqual(["closed"]);
  expect(log).toEqual([]);
});

test("a command that names no page opens nothing, and a refusal to bind is recorded", async () => {
  const quiet = deps();
  await openAfterAction(action, { result: {}, audit_id: 1, event_id: null, inverse: null }, quiet.d);
  expect(quiet.log).toEqual([`open_after ${action.command} {}`]);
  const { d, log } = deps();
  d.openAfter = async () => {
    throw new Error("its result has no `page` to open");
  };
  await openAfterAction(action, { result: {}, audit_id: 1, event_id: null, inverse: null }, d);
  expect(log).toEqual([`error Finish: its result has no \`page\` to open`]);
});

test("a command that asks returns what it asks about, then runs confirmed", async () => {
  const { d, log } = deps(
    new IpcCallError("`x` needs confirmation", "NEEDS_CONFIRMATION", null, ["process — runs programs"]),
  );
  expect(await performLensAction(action, run, null, null, false, d)).toEqual({ confirm: ["process — runs programs"] });
  expect(await performLensAction(action, run, null, null, true, d)).toBe("done");
  expect(log.filter((l) => l.startsWith("error"))).toEqual([]);
});

test("a refusal is recorded as the action's failure", async () => {
  const { d, log } = deps(new IpcCallError("denied: lens", "DENIED"));
  expect(await performLensAction(action, run, null, null, false, d)).toBe("failed");
  expect(log.at(-1)).toBe("error Finish: denied: lens");
});

test("copy puts the lens's text rendering on the clipboard", async () => {
  const log: string[] = [];
  const ok = await copyLens(run, "str1", {
    lensText: async (id, params, streamId) => `${id} ${JSON.stringify(params)} ${streamId}`,
    copyText: async (t) => void log.push(t),
    recordError: (l, m) => log.push(`error ${l} ${m}`),
  });
  expect(ok).toBe(true);
  expect(log).toEqual([`acme/prs {"repo":"x"} str1`]);
});

test("rowRecord maps columns to the row's values", () => {
  expect(rowRecord(["id", "title"], [7, "x"])).toEqual({ id: 7, title: "x" });
});

test("a row action with `when` is offered only on rows where that column is truthy", () => {
  const enable = { ...action, row: true, when: "can_enable" } as LensAction;
  expect(actionOffered(enable, { can_enable: 1 })).toBe(true);
  expect(actionOffered(enable, { can_enable: 0 })).toBe(false);
  expect(actionOffered(enable, { can_enable: null })).toBe(false);
  expect(actionOffered(enable, { can_enable: "" })).toBe(false);
  expect(actionOffered(enable, {})).toBe(false);
  expect(actionOffered({ ...action, row: true } as LensAction, {})).toBe(true);
});

// An action whose command can be undone says so: the toast offers Undo, as
// it does for a command a person runs from the launcher.
test("an undoable action's toast offers Undo, which undoes the run as the person", async () => {
  const log: string[] = [];
  const undoable = { result: {}, audit_id: 7, event_id: null, inverse: { name: "oxplow.agent_session.close", input: {} } } as never;
  const d: LensActionDeps = {
    runLensAction: async () => undoable,
    toast: (m, undo) => {
      log.push(`toast ${m} ${undo ? "with undo" : "without"}`);
      undo?.();
    },
    undo: async (auditId) => {
      log.push(`undo ${auditId}`);
    },
    recordError: () => {},
    openAfter: async () => null,
    openPage: () => {},
  };
  await performLensAction(action, run, null, null, false, d);
  expect(log).toEqual(["toast Finish: done. with undo", "undo 7"]);
});

// A form action whose form, less its fixed fields, is one text field is a
// prompt strip at the top of the lens; anything else is a slideover.
function start(properties: Record<string, unknown>, extra: Partial<FormStart> = {}): FormStart {
  return {
    command: { id: "c", input_schema: { type: "object", required: ["uri"], properties } },
    values: {},
    fixed: [],
    fields: null,
    choices: {},
    subfields: {},
    reads: [],
    ...extra,
  } as unknown as FormStart;
}

test("promptField is the one text field a form asks for, when that is all it asks", () => {
  expect(promptField(start({ to: { type: "string", description: "The new path" } }))?.key).toBe("to");
  expect(promptField(start({ uri: { type: "string" }, to: { type: "string" } }, { fixed: ["uri"] }))?.key).toBe("to");
  expect(promptField(start({ uri: { type: "string" }, title: { type: "string" }, notes: { type: "string" } }, { fields: ["title"] }))?.key).toBe("title");
});

test("promptField is null for more than one field, a fixed one, or a field that isn't text", () => {
  expect(promptField(start({ a: { type: "string" }, b: { type: "string" } }))).toBeNull();
  expect(promptField(start({ uri: { type: "string" } }, { fixed: ["uri"] }))).toBeNull();
  expect(promptField(start({ decision: { type: "string", enum: ["fix", "wontfix"] } }))).toBeNull();
  expect(promptField(start({ body: { type: "string", contentMediaType: "text/markdown" } }))).toBeNull();
  expect(promptField(start({ n: { type: "integer" } }))).toBeNull();
  // A field picked from a list is a select, which only the slideover draws.
  expect(promptField(start({ session: { type: "string" }, conversation: { type: "string" } }, { fixed: ["session"], choices: { conversation: [{ value: "c1", label: "One", group: null }] } }))).toBeNull();
});

// A secret (the schema's `writeOnly`, or a form field that says `secret`)
// is still one text field, masked and never required: blank keeps the
// saved one.
test("promptField says when its field is a secret", () => {
  expect(promptField(start({ value: { type: "string", writeOnly: true } }))).toMatchObject({ key: "value", secret: true });
  expect(promptField(start({ value: { type: "string" } }, { fields: [{ name: "value", secret: true }] }))).toMatchObject({ key: "value", secret: true });
  expect(promptField(start({ to: { type: "string" } }))).toMatchObject({ key: "to", secret: false });
});

test("a whole-lens action is offered as a button; one over the lens's one row only while it has exactly one and `when` holds", () => {
  const result = (rows: Array<Array<string | number | null>>) => ({ columns: ["title", "open"], rows }) as unknown as LensRun["result"];
  const plain: LensAction = { ...action };
  const oneRow: LensAction = { ...action, oneRow: true, when: "open" };
  // A row, group or edit action is not a whole-lens button.
  expect(offeredWholeLens({ ...action, row: true }, result([]))).toBe(false);
  expect(offeredWholeLens({ ...action, group: "x" }, result([]))).toBe(false);
  expect(offeredWholeLens({ ...action, edit: true }, result([]))).toBe(false);
  // An ordinary button needs no rows.
  expect(offeredWholeLens(plain, result([]))).toBe(true);
  expect(offeredWholeLens(plain, result([["a", 1], ["b", 1]]))).toBe(true);
  // The one-row action needs exactly one row, with its `when` truthy.
  expect(offeredWholeLens(oneRow, result([["a", 1]]))).toBe(true);
  expect(offeredWholeLens(oneRow, result([["a", 0]]))).toBe(false);
  expect(offeredWholeLens(oneRow, result([]))).toBe(false);
  expect(offeredWholeLens(oneRow, result([["a", 1], ["b", 1]]))).toBe(false);
  // Without a `when` it is offered on any single row.
  expect(offeredWholeLens({ ...oneRow, when: null }, result([["a", 0]]))).toBe(true);
});
