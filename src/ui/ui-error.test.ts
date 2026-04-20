import { describe, expect, test } from "bun:test";
import { runWithError, subscribeUiError } from "./ui-error.js";

describe("runWithError", () => {
  test("reports an error when given a thunk instead of a promise", async () => {
    const reports: string[] = [];
    const unsub = subscribeUiError((r) => reports.push(r.message));
    try {
      // Bug shape: caller wrapped the promise in an arrow function.
      runWithError("Save thing", (() => Promise.resolve(1)) as unknown as Promise<unknown>);
      expect(reports.length).toBe(1);
      expect(reports[0]).toContain("expected a Promise");
    } finally {
      unsub();
    }
  });

  test("forwards rejection to subscribers", async () => {
    const reports: string[] = [];
    const unsub = subscribeUiError((r) => reports.push(r.message));
    try {
      runWithError("Op", Promise.reject(new Error("nope")));
      await new Promise((r) => setTimeout(r, 0));
      expect(reports).toEqual(["nope"]);
    } finally {
      unsub();
    }
  });
});
