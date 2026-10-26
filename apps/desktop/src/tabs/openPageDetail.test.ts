import { describe, expect, test } from "bun:test";
import { createPageDetailStore } from "./openPageDetail.js";

describe("page detail store", () => {
  test("publishes per page id and notifies subscribers", () => {
    const store = createPageDetailStore();
    const seen: string[] = [];
    const off = store.subscribe(() => seen.push("x"));
    expect(store.get("oxplow:oxplow/page?page=a/b")).toBeNull();
    store.publish("oxplow:oxplow/page?page=a/b", { pageId: "a/b", params: { k: 1 } });
    expect(store.get("oxplow:oxplow/page?page=a/b")).toEqual({ pageId: "a/b", params: { k: 1 } });
    expect(store.get("oxplow:oxplow/page?page=other")).toBeNull();
    store.publish("oxplow:oxplow/page?page=a/b", null);
    expect(store.get("oxplow:oxplow/page?page=a/b")).toBeNull();
    off();
    store.publish("oxplow:oxplow/page?page=a/b", { x: 1 });
    expect(seen).toHaveLength(2);
  });
});
