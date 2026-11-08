import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { OpErrorInput } from "../opErrors.js";
import { createFailureStore, createInlineClaims } from "./actionFailures.js";
import { classifyFailure, createRunner } from "./runForPerson.js";

function setup() {
  const reports: OpErrorInput[] = [];
  const failures = createFailureStore();
  const later = (f: () => void) => void Promise.resolve().then(f);
  const claims = createInlineClaims(later, 0);
  const runner = createRunner({
    report: async (entry) => {
      reports.push(entry);
      return `ev${reports.length}`;
    },
    failures,
    claims,
    now: () => 42,
  });
  return { reports, failures, claims, runner };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

test("each kind of failure is told apart", () => {
  expect(classifyFailure(new IpcCallError("title is empty", "INVALID", "/title"))).toEqual({ kind: "field", errors: { title: "title is empty" } });
  expect(classifyFailure(new IpcCallError("asks", "NEEDS_CONFIRMATION", null, ["sql — may run SQL"]))).toEqual({
    kind: "confirm",
    details: ["sql — may run SQL"],
  });
  expect(classifyFailure(new IpcCallError("closed", "CANCELLED"))).toEqual({ kind: "cancelled" });
  // A component's rejection is a plain object, not an Error.
  expect(classifyFailure({ code: "CANCELLED", message: "closed" })).toEqual({ kind: "cancelled" });
  expect(classifyFailure(new IpcCallError("held", "INVALID"))).toEqual({ kind: "failed", message: "held", code: "INVALID" });
  expect(classifyFailure(new Error("boom"))).toEqual({ kind: "failed", message: "boom", code: null });
  expect(classifyFailure({ code: "FAILED", message: "nope" })).toEqual({ kind: "failed", message: "nope", code: "FAILED" });
});

test("a failure is recorded once and shown at the control that ran it", async () => {
  const { reports, failures, claims, runner } = setup();
  const off = failures.mount("action/boom");
  const out = await runner.run("Boom", () => Promise.reject(new IpcCallError("it broke", "FAILED")), {
    anchor: "action/boom",
    command: "e2e.fail.now",
  });
  expect(out).toEqual({ ok: false, failure: { kind: "failed", message: "it broke", code: "FAILED" } });
  expect(reports).toEqual([{ label: "Boom", message: "it broke", command: "e2e.fail.now" }]);
  await flush();
  expect(failures.get("action/boom")).toEqual({ message: "it broke", code: "FAILED", errorId: "ev1", at: 42 });
  await claims.settled();
  expect(claims.shown("ev1")).toBe(true);
  off();
});

// The record links what recorded the run: the audit row the daemon's error
// names, and an approval's proposal.
test("a failure's record names the run's audit row and the proposal it decided", async () => {
  const { reports, runner } = setup();
  await runner.run("Approve: Delete", () => Promise.reject(new IpcCallError("gone", "INVALID", null, [], 41)), {
    command: "oxplow.work_item.delete",
    proposal: 3,
  });
  expect(reports).toEqual([{ label: "Approve: Delete", message: "gone", command: "oxplow.work_item.delete", auditId: 41, proposalId: 3 }]);
});

test("a failure with no control on screen is left to the toast", async () => {
  const { reports, claims, runner } = setup();
  await runner.run("Close", () => Promise.reject(new Error("gone")), { anchor: "action/close" });
  await runner.run("Pull", () => Promise.reject(new Error("offline")));
  expect(reports.map((r) => r.label)).toEqual(["Close", "Pull"]);
  await flush();
  await claims.settled();
  expect(claims.shown("ev1")).toBe(false);
  expect(claims.shown("ev2")).toBe(false);
});

test("a field error is a failure where no field shows it", async () => {
  const { reports, runner } = setup();
  const out = await runner.run("Finish", () => Promise.reject(new IpcCallError("required", "INVALID", "/title")));
  expect(out).toEqual({ ok: false, failure: { kind: "failed", message: "required", code: "INVALID" } });
  expect(reports.map((r) => r.message)).toEqual(["required"]);
});

test("a field error a form shows, a confirmation and a cancel are not recorded", async () => {
  const { reports, failures, runner } = setup();
  failures.mount("form/x");
  const field = await runner.run("Save", () => Promise.reject(new IpcCallError("required", "INVALID", "/title")), {
    anchor: "form/x",
    showsFields: true,
  });
  expect(field).toEqual({ ok: false, failure: { kind: "field", errors: { title: "required" } } });
  await runner.run("Save", () => Promise.reject(new IpcCallError("asks", "NEEDS_CONFIRMATION")), { anchor: "form/x" });
  await runner.run("Save", () => Promise.reject({ code: "CANCELLED", message: "closed" }), { anchor: "form/x" });
  expect(reports).toEqual([]);
  expect(failures.get("form/x")).toBeNull();
});

test("running again clears the control's last failure; success keeps it clear", async () => {
  const { failures, runner } = setup();
  failures.mount("a");
  await runner.run("A", () => Promise.reject(new Error("x")), { anchor: "a" });
  expect(failures.get("a")?.message).toBe("x");
  const ok = await runner.run("A", async () => 7, { anchor: "a" });
  expect(ok).toEqual({ ok: true, value: 7 });
  expect(failures.get("a")).toBeNull();
});

test("a run that asks first runs again confirmed once the person says yes, with what it asks about", async () => {
  const { runner } = setup();
  const calls: boolean[] = [];
  const asked: string[][] = [];
  const call = async (confirmed: boolean) => {
    calls.push(confirmed);
    if (!confirmed) throw new IpcCallError("asks", "NEEDS_CONFIRMATION", null, ["sql"]);
    return "ran";
  };
  const yes = await runner.runAsking("Deploy", call, async (details) => (asked.push(details), true));
  expect(yes).toEqual({ ok: true, value: "ran" });
  expect(calls).toEqual([false, true]);
  expect(asked).toEqual([["sql"]]);
  const no = await runner.runAsking("Deploy", call, async () => false);
  expect(no).toEqual({ ok: false, failure: { kind: "cancelled" } });
});
