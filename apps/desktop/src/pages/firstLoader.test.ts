import { describe, expect, test } from "bun:test";
import { firstLoader } from "./firstLoader.js";
import { decl } from "./testing/fakeRun.js";

const content = { ...decl("content"), loader: { kind: "content" as const, text: false, max_bytes: 65536 } };

describe("firstLoader", () => {
  test("is the first loader the author declared, not the system's content", () => {
    expect(firstLoader({ loaders: [content, decl("row"), decl("other")] })?.name).toBe("row");
  });
  test("is the content loader of a page that declares none", () => {
    expect(firstLoader({ loaders: [content] })?.name).toBe("content");
  });
  test("is the first loader of a standalone page", () => {
    expect(firstLoader({ loaders: [decl("a"), decl("b")] })?.name).toBe("a");
    expect(firstLoader({ loaders: [] })).toBeUndefined();
  });
});
