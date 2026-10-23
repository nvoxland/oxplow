import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { LensAction, LensRun } from "../tauri-bridge/generated/bindings.js";
import { actionOffered, addLensToContext, copyLens, openAfterAction, performLensAction, rowRecord, type LensActionDeps } from "./lensActions.js";

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

test("add to context hands the lens and its params to the agent", () => {
  const log: string[] = [];
  addLensToContext(run, (t) => log.push(t));
  expect(log).toEqual(['[oxplow lens acme/prs repo="x"] ']);
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
