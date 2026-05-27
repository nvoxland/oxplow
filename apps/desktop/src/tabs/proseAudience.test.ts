import { describe, expect, test } from "bun:test";
import { createProseAudienceStore, DEFAULT_PROSE_AUDIENCE } from "./proseAudience.js";

function memStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      map.set(k, v);
    },
    removeItem: (k: string) => {
      map.delete(k);
    },
    _map: map,
  };
}

describe("proseAudience store", () => {
  test("defaults to developer for unknown and null page keys", () => {
    const s = createProseAudienceStore(memStorage());
    expect(s.get("t-1::tab-1")).toBe("developer");
    expect(s.get(null)).toBe(DEFAULT_PROSE_AUDIENCE);
  });

  test("per-page scoping — two keys are independent", () => {
    const s = createProseAudienceStore(memStorage());
    s.set("t-1::wiki", "caveman");
    s.set("t-1::task", "executive");
    expect(s.get("t-1::wiki")).toBe("caveman");
    expect(s.get("t-1::task")).toBe("executive");
  });

  test("setting developer removes the row but get still returns developer", () => {
    const store = memStorage();
    const s = createProseAudienceStore(store);
    s.set("p", "executive");
    expect(store._map.size).toBe(1);
    s.set("p", "developer");
    // blob is dropped entirely when no non-default entries remain
    expect(store._map.size).toBe(0);
    expect(s.get("p")).toBe("developer");
  });

  test("malformed / invalid values fall back to developer", () => {
    const store = memStorage();
    store.setItem("oxplow.prose-audience.v1", "{not json");
    const s = createProseAudienceStore(store);
    expect(s.get("p")).toBe("developer");

    const store2 = memStorage();
    store2.setItem("oxplow.prose-audience.v1", JSON.stringify({ p: "martian" }));
    const s2 = createProseAudienceStore(store2);
    expect(s2.get("p")).toBe("developer");
  });

  test("clear drops only the named key", () => {
    const s = createProseAudienceStore(memStorage());
    s.set("a", "caveman");
    s.set("b", "executive");
    s.clear("a");
    expect(s.get("a")).toBe("developer");
    expect(s.get("b")).toBe("executive");
  });

  test("subscribe fires on set and clear", () => {
    const s = createProseAudienceStore(memStorage());
    let n = 0;
    const off = s.subscribe(() => {
      n++;
    });
    s.set("p", "caveman");
    s.clear("p");
    expect(n).toBe(2);
    off();
    s.set("p", "executive");
    expect(n).toBe(2);
  });

  test("set is a no-op when value is unchanged (no spurious notify)", () => {
    const s = createProseAudienceStore(memStorage());
    s.set("p", "caveman");
    let n = 0;
    const off = s.subscribe(() => {
      n++;
    });
    s.set("p", "caveman");
    expect(n).toBe(0);
    off();
  });
});
