import { expect, test } from "bun:test";

import type { SearchHit } from "../../api.js";
import { fileOptions, mergeRefOptions } from "./refOptions.js";

// What `[[` lists: files by name (a name that starts with the query first)
// and the site search's titled things; a file's content match isn't a name.

test("files match every word of the query: a name starting with one first, then a name holding one", () => {
  const files = ["src/app/Notes.tsx", "src/notes/index.ts", "docs/release-notes.md", "src/other.ts"];
  expect(fileOptions(files, "notes", 10).map((o) => o.title)).toEqual(["src/app/Notes.tsx", "docs/release-notes.md", "src/notes/index.ts"]);
  expect(fileOptions(files, "src notes", 10).map((o) => o.title)).toEqual(["src/app/Notes.tsx", "src/notes/index.ts"]);
  expect(fileOptions(files, "", 10)).toEqual([]);
});

test("search hits come first, without file content matches, then files", () => {
  const hit = (resource: string, uri: string, title: string): SearchHit => ({ resource, uri, title, stream_id: null, snippet: "", score: 1 });
  const merged = mergeRefOptions(
    [hit("oxplow/wiki", "oxplow:oxplow/wiki?slug=notes", "Notes"), hit("oxplow/file", "oxplow:oxplow/file?path=a.ts", "a.ts")],
    fileOptions(["src/notes.ts"], "notes", 10),
    10,
  );
  expect(merged.map((o) => [o.kind, o.title])).toEqual([
    ["wiki", "Notes"],
    ["file", "src/notes.ts"],
  ]);
});
