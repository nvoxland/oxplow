import { describe, expect, it } from "bun:test";
import { createMemoizedScanner, findFilePathMatches } from "./terminal-link-provider.js";

describe("findFilePathMatches", () => {
  it("finds a relative path with extension", () => {
    const m = findFilePathMatches("see apps/desktop/src/App.tsx for details");
    expect(m).toEqual([
      { start: 4, end: 28, text: "apps/desktop/src/App.tsx", line: undefined, column: undefined },
    ]);
  });

  it("captures :line suffix", () => {
    const m = findFilePathMatches("apps/desktop/src/App.tsx:42 broken");
    expect(m).toEqual([
      { start: 0, end: 27, text: "apps/desktop/src/App.tsx", line: 42, column: undefined },
    ]);
  });

  it("captures :line:col suffix", () => {
    const m = findFilePathMatches("crates/oxplow-app/src/lib.rs:120:8");
    expect(m).toEqual([
      { start: 0, end: 34, text: "crates/oxplow-app/src/lib.rs", line: 120, column: 8 },
    ]);
  });

  it("strips trailing prose punctuation", () => {
    const m = findFilePathMatches("see apps/desktop/src/App.tsx:627.");
    expect(m).toEqual([
      { start: 4, end: 32, text: "apps/desktop/src/App.tsx", line: 627, column: undefined },
    ]);
  });

  it("handles absolute paths", () => {
    const m = findFilePathMatches("/etc/hosts is real");
    expect(m).toEqual([
      { start: 0, end: 10, text: "/etc/hosts", line: undefined, column: undefined },
    ]);
  });

  it("handles ./ and ../ prefixes", () => {
    const m = findFilePathMatches("./foo.ts and ../bar.rs");
    expect(m.map((x) => x.text)).toEqual(["./foo.ts", "../bar.rs"]);
  });

  it("matches a bare extensioned filename", () => {
    const m = findFilePathMatches("edit Cargo.toml please");
    expect(m).toEqual([
      { start: 5, end: 15, text: "Cargo.toml", line: undefined, column: undefined },
    ]);
  });

  it("rejects URLs", () => {
    const m = findFilePathMatches("see https://example.com/foo.html for context");
    expect(m).toEqual([]);
  });

  it("drops path-shaped tokens that validatePath rejects", () => {
    // A plugin name in prose looks like name.ext but isn't a real file.
    const validate = (p: string) => p === "apps/desktop/src/App.tsx";
    const prose = findFilePathMatches("the oxplow.junit plugin", validate);
    expect(prose).toEqual([]);
    const real = findFilePathMatches("see apps/desktop/src/App.tsx now", validate);
    expect(real.map((x) => x.text)).toEqual(["apps/desktop/src/App.tsx"]);
  });

  it("validatePath gets the bare path without the line:col suffix", () => {
    const seen: string[] = [];
    findFilePathMatches("apps/desktop/src/App.tsx:42 here", (p) => {
      seen.push(p);
      return true;
    });
    expect(seen).toEqual(["apps/desktop/src/App.tsx"]);
  });

  it("rejects bare numbers and version-y dots", () => {
    const m = findFilePathMatches("version 1.5 of x has 3.14 problems");
    expect(m).toEqual([]);
  });

  it("rejects time-of-day", () => {
    const m = findFilePathMatches("ran at 12:34 today");
    expect(m).toEqual([]);
  });

  it("finds multiple paths on a line", () => {
    const m = findFilePathMatches("moved foo/a.ts to bar/b.ts:10");
    expect(m.map((x) => `${x.text}:${x.line ?? ""}`)).toEqual([
      "foo/a.ts:",
      "bar/b.ts:10",
    ]);
  });

  it("excludes surrounding quote chars", () => {
    const m = findFilePathMatches('opened "apps/desktop/src/App.tsx" already');
    expect(m).toEqual([
      { start: 8, end: 32, text: "apps/desktop/src/App.tsx", line: undefined, column: undefined },
    ]);
  });

  it("excludes a trailing closing paren", () => {
    const m = findFilePathMatches("(see apps/desktop/src/App.tsx:42)");
    expect(m).toEqual([
      { start: 5, end: 32, text: "apps/desktop/src/App.tsx", line: 42, column: undefined },
    ]);
  });

  it("handles backticked paths", () => {
    const m = findFilePathMatches("the `apps/desktop/src/App.tsx` file");
    expect(m).toEqual([
      { start: 5, end: 29, text: "apps/desktop/src/App.tsx", line: undefined, column: undefined },
    ]);
  });

  it("ignores http path with no scheme prefix when colon-port-y", () => {
    // `localhost:3000` should NOT match — no slash, no extension.
    const m = findFilePathMatches("listening on localhost:3000");
    expect(m).toEqual([]);
  });

  it("rejects email-shaped tokens", () => {
    const m = findFilePathMatches("contact nathan@voxland.net for info");
    expect(m).toEqual([]);
  });
});

describe("createMemoizedScanner", () => {
  it("scans a text once, then serves repeats from cache (same reference)", () => {
    let calls = 0;
    const scan = createMemoizedScanner((t) => {
      calls++;
      return findFilePathMatches(t);
    });
    const a = scan("open src/main.rs:10 now");
    const b = scan("open src/main.rs:10 now");
    expect(calls).toBe(1); // second call was a cache hit
    expect(b).toBe(a); // same cached array, not a recompute
    expect(a).toEqual([{ start: 5, end: 19, text: "src/main.rs", line: 10, column: undefined }]);
  });

  it("recomputes for different text", () => {
    let calls = 0;
    const scan = createMemoizedScanner((t) => {
      calls++;
      return findFilePathMatches(t);
    });
    scan("a.ts");
    scan("b.ts");
    expect(calls).toBe(2);
  });

  it("is bounded — evicts the oldest entry past the cap", () => {
    let calls = 0;
    const scan = createMemoizedScanner(() => {
      calls++;
      return [];
    }, 2);
    scan("1"); // miss (cache [1])
    scan("2"); // miss (cache [1,2])
    scan("3"); // miss → evicts "1" (cache [2,3])
    scan("2"); // hit
    expect(calls).toBe(3);
    scan("1"); // "1" was evicted → recompute
    expect(calls).toBe(4);
  });

  it("is a true LRU — a hit refreshes recency so it survives eviction", () => {
    let calls = 0;
    const scan = createMemoizedScanner(() => {
      calls++;
      return [];
    }, 2);
    scan("a"); // [a]
    scan("b"); // [a,b]
    scan("a"); // hit → refresh recency → [b,a]
    scan("c"); // miss → evict LRU "b" (not "a") → [a,c]
    scan("a"); // still cached because it was recently used
    // Under FIFO "a" would have been evicted by "c" and recomputed here.
    expect(calls).toBe(3); // only a, b, c ever computed
  });
});
