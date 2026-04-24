import { describe, expect, test } from "bun:test";
import { parseNoteRefs } from "./wiki-note-refs.js";

describe("parseNoteRefs", () => {
  test("returns [] for empty or refless body", () => {
    expect(parseNoteRefs("")).toEqual([]);
    expect(parseNoteRefs("just prose with no paths")).toEqual([]);
  });

  test("extracts path-shaped tokens from prose", () => {
    const refs = parseNoteRefs("See src/electron/runtime.ts for the answer.");
    expect(refs.map((r) => r.path)).toEqual(["src/electron/runtime.ts"]);
  });

  test("strips trailing :Symbol anchors but keeps the path", () => {
    const refs = parseNoteRefs("Look at src/git/git.ts:readWorktreeHeadSha");
    expect(refs.map((r) => r.path)).toEqual(["src/git/git.ts"]);
  });

  test("extracts paths from fenced block info strings", () => {
    const body = [
      "```ts src/persistence/wiki-note-store.ts",
      "export class Foo {}",
      "```",
    ].join("\n");
    const refs = parseNoteRefs(body);
    expect(refs.map((r) => r.path)).toEqual(["src/persistence/wiki-note-store.ts"]);
  });

  test("dedupes repeated references", () => {
    const body = "src/a/b.ts is referenced again at src/a/b.ts and src/a/b.ts:Sym";
    const refs = parseNoteRefs(body);
    expect(refs.map((r) => r.path)).toEqual(["src/a/b.ts"]);
  });

  test("ignores URLs and fragments that look like paths", () => {
    const refs = parseNoteRefs("link https://example.com/foo.html and /absolute/path.ts");
    // No rooted / URL paths in v1 — only project-relative.
    expect(refs.map((r) => r.path)).toEqual([]);
  });

  test("extracts multiple distinct paths", () => {
    const body = "src/foo.ts calls into src/bar/baz.tsx and tests live in src/foo.test.ts";
    const paths = parseNoteRefs(body).map((r) => r.path).sort();
    expect(paths).toEqual(["src/bar/baz.tsx", "src/foo.test.ts", "src/foo.ts"]);
  });
});
