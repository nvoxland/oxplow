import { describe, expect, test } from "bun:test";
import { latestWins } from "./latestWins.js";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("latestWins", () => {
  test("an older response that lands last is dropped", async () => {
    const calls = [deferred<string>(), deferred<string>()];
    let i = 0;
    const seen: string[] = [];
    const r = latestWins(() => calls[i++].promise, (v) => seen.push(v), () => {});
    r.run();
    r.run();
    calls[1].resolve("new");
    calls[0].resolve("old");
    await Promise.resolve();
    await Promise.resolve();
    expect(seen).toEqual(["new"]);
  });

  test("the newest run's error is reported; nothing arrives after close", async () => {
    const calls = [deferred<string>(), deferred<string>()];
    let i = 0;
    const seen: unknown[] = [];
    const errors: unknown[] = [];
    const r = latestWins(() => calls[i++].promise, (v) => seen.push(v), (e) => errors.push(e));
    r.run();
    calls[0].reject("boom");
    await Promise.resolve();
    await Promise.resolve();
    expect(errors).toEqual(["boom"]);
    r.run();
    r.close();
    calls[1].resolve("late");
    await Promise.resolve();
    await Promise.resolve();
    expect(seen).toEqual([]);
  });
});
