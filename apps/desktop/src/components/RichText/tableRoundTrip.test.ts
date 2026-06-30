import { afterEach, expect, test } from "bun:test";
import { Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "tiptap-markdown";
import Table from "@tiptap/extension-table";
import TableRow from "@tiptap/extension-table-row";
import TableHeader from "@tiptap/extension-table-header";
import TableCell from "@tiptap/extension-table-cell";

// Regression: the wiki body is an always-on Tiptap editor, so a GFM table
// must survive a markdown → editor → markdown round-trip (otherwise opening
// + autosaving a page with tables flattens them on disk). tiptap-markdown
// ships the GFM table serializer; this proves it engages once the standard
// table nodes are in the schema.

let editor: Editor | null = null;
afterEach(() => {
  editor?.destroy();
  editor = null;
});

function roundTrip(md: string): string {
  editor = new Editor({
    extensions: [
      StarterKit,
      Markdown.configure({ html: false, breaks: false }),
      Table,
      TableRow,
      TableHeader,
      TableCell,
    ],
    content: md,
  });
  return editor.storage.markdown.getMarkdown();
}

test("a GFM table round-trips through the editor without flattening", () => {
  const md = ["| Column | Notes |", "| --- | --- |", "| id | PK |", "| key | unique |"].join("\n");
  const out = roundTrip(md);
  // Header, separator, and body rows all survive as pipe-delimited rows.
  expect(out).toContain("| Column | Notes |");
  expect(out).toMatch(/\|\s*-{3,}\s*\|/);
  expect(out).toContain("| id | PK |");
  expect(out).toContain("| key | unique |");
});

test("inline code inside a table cell survives the round-trip", () => {
  const md = ["| Field | Type |", "| --- | --- |", "| `id` | PK |"].join("\n");
  const out = roundTrip(md);
  expect(out).toContain("`id`");
  expect(out).toMatch(/\|\s*-{3,}\s*\|/);
});
