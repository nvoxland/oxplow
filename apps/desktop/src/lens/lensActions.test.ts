import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { LensAction, LensRun } from "../tauri-bridge/generated/bindings.js";
import { addLensToContext, copyLens, performLensAction, rowRecord, type LensActionDeps } from "./lensActions.js";

const run = { lens: { id: "acme/prs" }, params: { repo: "x" } } as unknown as LensRun;
const action: LensAction = { id: "finish", label: "Finish", command: "oxplow.work_item.transition", input: {}, row: false };

function deps(fail?: Error) {
  const log: string[] = [];
  const d: LensActionDeps = {
    runLensAction: async (id, a, params, row, streamId, confirmed) => {
      log.push(`run ${id} ${a} ${JSON.stringify(params)} ${JSON.stringify(row)} ${streamId} ${confirmed}`);
      if (fail && !(confirmed && fail instanceof IpcCallError && fail.code === "NEEDS_CONFIRMATION")) throw fail;
      return { result: {}, audit_id: 1, event_id: null, inverse: null };
    },
    toast: (m) => log.push(`toast ${m}`),
    recordError: (label, m) => log.push(`error ${label}: ${m}`),
  };
  return { d, log };
}

test("an action runs its command with the run's params and row", async () => {
  const { d, log } = deps();
  expect(await performLensAction(action, run, { id: 7 }, "str1", false, d)).toBe("done");
  expect(log).toEqual([`run acme/prs finish {"repo":"x"} {"id":7} str1 false`, "toast Finish: done."]);
});

test("a command that asks returns needs-confirmation, then runs confirmed", async () => {
  const { d, log } = deps(new IpcCallError("`x` needs confirmation", "NEEDS_CONFIRMATION"));
  expect(await performLensAction(action, run, null, null, false, d)).toBe("needs-confirmation");
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
