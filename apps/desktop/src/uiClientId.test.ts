import { expect, test } from "bun:test";
import { CLIENT_ID_KEY, createUiClientId } from "./uiClientId.js";

test("a window reuses its session identity for logs, reports and event reconnects", () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
  const first = createUiClientId(storage, () => "window-one");
  expect(first()).toBe("window-one");
  expect(first()).toBe("window-one");
  expect(values.get(CLIENT_ID_KEY)).toBe("window-one");
  expect(createUiClientId(storage, () => "unexpected")()).toBe("window-one");
});

test("unavailable storage still gives one stable window identity", () => {
  let generated = 0;
  const id = createUiClientId({ getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } }, () => `fallback-${++generated}`);
  expect(id()).toBe("fallback-1");
  expect(id()).toBe("fallback-1");
  expect(generated).toBe(1);
});
