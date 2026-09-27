import { describe, expect, test } from "bun:test";
import { createPageDetailStore } from "./openPageDetail.js";

describe("page detail store", () => {
  test("publishes per page id and notifies subscribers", () => {
    const store = createPageDetailStore();
    const seen: string[] = [];
    const off = store.subscribe(() => seen.push("x"));
    expect(store.get("lens:a/b")).toBeNull();
    store.publish("lens:a/b", { lensId: "a/b", params: { k: 1 } });
    expect(store.get("lens:a/b")).toEqual({ lensId: "a/b", params: { k: 1 } });
    expect(store.get("lens:other")).toBeNull();
    store.publish("lens:a/b", null);
    expect(store.get("lens:a/b")).toBeNull();
    off();
    store.publish("lens:a/b", { x: 1 });
    expect(seen).toHaveLength(2);
  });
});
