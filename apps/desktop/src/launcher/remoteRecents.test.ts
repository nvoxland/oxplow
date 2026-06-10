import { describe, expect, test } from "bun:test";

import {
  forgetRemote,
  loadRecentRemotes,
  normalizeBase,
  rememberRemote,
  type StringStorage,
} from "./remoteRecents.js";

function memStorage(initial: Record<string, string> = {}): StringStorage {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
  };
}

describe("normalizeBase", () => {
  test("adds http scheme when missing", () => {
    expect(normalizeBase("127.0.0.1:7420")).toBe("http://127.0.0.1:7420");
  });

  test("keeps explicit scheme and strips trailing slashes", () => {
    expect(normalizeBase("https://box:7420//")).toBe("https://box:7420");
  });

  test("trims whitespace", () => {
    expect(normalizeBase("  http://h:1 ")).toBe("http://h:1");
  });
});

describe("recent remotes list", () => {
  test("empty storage yields empty list", () => {
    expect(loadRecentRemotes(memStorage())).toEqual([]);
  });

  test("malformed storage yields empty list", () => {
    const s = memStorage({ "oxplow.recentRemotes": "not json" });
    expect(loadRecentRemotes(s)).toEqual([]);
  });

  test("remember puts newest first and dedupes by base", () => {
    const s = memStorage();
    rememberRemote(s, "http://a:1", 100);
    rememberRemote(s, "http://b:2", 200);
    const next = rememberRemote(s, "http://a:1", 300);
    expect(next.map((r) => r.base)).toEqual(["http://a:1", "http://b:2"]);
    expect(next[0].lastConnectedAt).toBe(300);
  });

  test("remember normalizes the base before storing", () => {
    const s = memStorage();
    const next = rememberRemote(s, "a:1//", 1);
    expect(next[0].base).toBe("http://a:1");
  });

  test("caps the list at five entries", () => {
    const s = memStorage();
    for (let i = 0; i < 7; i++) rememberRemote(s, `http://h:${i}`, i);
    const list = loadRecentRemotes(s);
    expect(list).toHaveLength(5);
    expect(list[0].base).toBe("http://h:6");
  });

  test("forget removes the entry", () => {
    const s = memStorage();
    rememberRemote(s, "http://a:1", 1);
    rememberRemote(s, "http://b:2", 2);
    const next = forgetRemote(s, "http://a:1");
    expect(next.map((r) => r.base)).toEqual(["http://b:2"]);
  });
});
