import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
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

  test("returns quickly on adversarial dot/slash-heavy input", () => {
    // Both the inner path-segment class `[a-zA-Z0-9_.-]+` and the trailing
    // `\.[a-zA-Z0-9]{1,6}` consume dots, so a body full of dotted segments
    // could in principle trigger backtracking. Pin linear-ish behaviour.
    let body = "see ";
    for (let i = 0; i < 50; i++) body += "/a.b.c.d.e.f.g.h.i.j.k";
    body += "/zzzzzzzzzzz "; // ext too long → regex must give up
    const start = Date.now();
    parseNoteRefs(body.repeat(20));
    expect(Date.now() - start).toBeLessThan(250);
  });

  test("returns quickly on real-world doc content (DEV.md)", () => {
    // Regression guard for "Opening DEV.md locks up the app". The parser is on
    // the hot path for every `.oxplow/notes/<slug>.md` watcher event; pasting
    // a doc-style body with many fenced blocks and bracketed relative links
    // must not trip catastrophic backtracking.
    const repoRoot = join(import.meta.dir, "..", "..");
    const body = readFileSync(join(repoRoot, "DEV.md"), "utf8");
    const start = Date.now();
    const refs = parseNoteRefs(body);
    const elapsed = Date.now() - start;
    expect(elapsed).toBeLessThan(250);
    // Should at least find the referenced .context docs.
    const paths = refs.map((r) => r.path);
    expect(paths.some((p) => p.endsWith("architecture.md"))).toBe(true);
  });
});
