import { describe, expect, test } from "bun:test";
import { agentRef, fileRef, wikiPageRef, taskRef } from "./pageRefs.js";
import { resolveActiveTabRef } from "./resolveActiveTabRef.js";

describe("resolveActiveTabRef", () => {
  test("agent id resolves to agentRef", () => {
    expect(resolveActiveTabRef("agent", [], [])).toEqual(agentRef());
  });

  test("matching pageTab id returns that ref", () => {
    const note = wikiPageRef("data-model");
    const work = taskRef("1");
    const got = resolveActiveTabRef(note.id, [work, note], []);
    expect(got).toBe(note);
  });

  test("file:<path> resolves when path is in openOrder", () => {
    const got = resolveActiveTabRef("file:src/a.ts", [], ["src/a.ts", "src/b.ts"]);
    expect(got).toEqual(fileRef("src/a.ts"));
  });

  test("file:<path> returns null when path is not open", () => {
    expect(resolveActiveTabRef("file:src/missing.ts", [], ["src/a.ts"])).toBeNull();
  });

  test("unknown id returns null", () => {
    expect(resolveActiveTabRef("nope", [], [])).toBeNull();
  });
});
