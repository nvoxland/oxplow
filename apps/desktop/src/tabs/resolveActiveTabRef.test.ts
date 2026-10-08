import { describe, expect, test } from "bun:test";
import { agentSessionRef, fileRef, wikiPageRef, workItemTabRef } from "./pageRefs.js";
import { resolveActiveTabRef } from "./resolveActiveTabRef.js";

describe("resolveActiveTabRef", () => {
  test("an agent session's tab resolves from the page tabs like any other", () => {
    const session = agentSessionRef("ses3");
    expect(resolveActiveTabRef(session.id, [session], [])).toBe(session);
  });

  test("matching pageTab id returns that ref", () => {
    const note = wikiPageRef("data-model");
    const work = workItemTabRef("work_item:oxplow:tsk1");
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

  test("a pinned-revision file id is not the open editor file", () => {
    expect(resolveActiveTabRef("file:src/a.ts@git:HEAD", [], ["src/a.ts"])).toBeNull();
  });

  test("unknown id returns null", () => {
    expect(resolveActiveTabRef("nope", [], [])).toBeNull();
  });
});
