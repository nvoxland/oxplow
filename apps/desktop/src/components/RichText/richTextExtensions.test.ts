import { afterEach, expect, test } from "bun:test";
import { Editor } from "@tiptap/react";
import { getMarkdown, richTextExtensions } from "./richTextExtensions.js";

// These run against the exact extension list RichTextField mounts, so a
// dependency upgrade that changes what a bundle (StarterKit) registers shows
// up here rather than as a silently-different editor.

let editor: Editor | null = null;
afterEach(() => {
  editor?.destroy();
  editor = null;
});

function mount(md: string, opts: { inlineOnly?: boolean } = {}): Editor {
  editor = new Editor({ extensions: richTextExtensions(opts), content: md });
  return editor;
}

test("each extension is registered exactly once", () => {
  // StarterKit 3 bundles Link; InternalLink must replace it, not sit beside it.
  const names = mount("x").extensionManager.extensions.map((e) => e.name);
  expect(names.filter((n) => n === "link")).toHaveLength(1);
  expect(names.filter((n) => n === "codeBlock")).toHaveLength(1);
  expect(new Set(names).size).toBe(names.length);
});

test("underline is not in the schema — markdown has no syntax for it", () => {
  // StarterKit 3 bundles Underline; with html:false it can't round-trip, so
  // Cmd+U would produce formatting that silently vanishes on save.
  expect(mount("x").schema.marks.underline).toBeUndefined();
});

test("internal-scheme links survive the round-trip as links", () => {
  const md = "See [the file](file:src/a.ts) and [a task](task:42).";
  expect(getMarkdown(mount(md)).trim()).toBe(md);
});

test("a document ending in a code block round-trips unchanged", () => {
  const md = "Intro\n\n```ts\nconst x = 1;\n```";
  expect(getMarkdown(mount(md)).trim()).toBe(md);
});

test("inline-only fields have no block nodes in the schema", () => {
  const { nodes } = mount("x", { inlineOnly: true }).schema;
  for (const n of ["heading", "bulletList", "orderedList", "blockquote", "horizontalRule", "table"]) {
    expect(nodes[n]).toBeUndefined();
  }
});
