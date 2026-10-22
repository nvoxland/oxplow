import type { Editor, Extensions } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Table, TableCell, TableHeader, TableRow } from "@tiptap/extension-table";
import { Markdown, type MarkdownStorage } from "tiptap-markdown";
import { InternalLink } from "./InternalLink.js";
import { MermaidBlock } from "./MermaidBlock.js";
import { CommentDecorations } from "./CommentDecorations.js";

// tiptap-markdown doesn't augment Tiptap 3's typed `Storage`, so
// `editor.storage.markdown` would otherwise be unknown.
declare module "@tiptap/core" {
  interface Storage {
    markdown: MarkdownStorage;
  }
}

export function getMarkdown(editor: Editor): string {
  return editor.storage.markdown?.getMarkdown() ?? "";
}

/**
 * The extension list every `RichTextField` mounts. Lives outside the
 * component so tests exercise the real configuration, not a copy.
 */
export function richTextExtensions({
  inlineOnly = false,
  placeholder = "",
}: { inlineOnly?: boolean; placeholder?: string } = {}): Extensions {
  return [
    StarterKit.configure({
      // Replaced by MermaidBlock (which `extend`s CodeBlock under the
      // same name "codeBlock"). Avoid the duplicate name warning.
      codeBlock: false,
      // Replaced by InternalLink (StarterKit bundles Link since Tiptap 3).
      link: false,
      // Markdown has no underline syntax and we parse/serialize with
      // html:false, so an underline mark would vanish on save.
      underline: false,
      // Inline-only fields skip block features at the schema level.
      heading: inlineOnly ? false : undefined,
      bulletList: inlineOnly ? false : undefined,
      orderedList: inlineOnly ? false : undefined,
      blockquote: inlineOnly ? false : undefined,
      horizontalRule: inlineOnly ? false : undefined,
    }),
    MermaidBlock,
    InternalLink,
    // GFM tables (block content — off for inline-only fields). tiptap-markdown
    // ships the GFM table serializer + markdown-it parses tables, so adding
    // the standard table nodes is all the round-trip needs. Without these the
    // editor flattens any table in a wiki page to run-on text on autosave.
    ...(inlineOnly ? [] : [Table.configure({ resizable: true }), TableRow, TableHeader, TableCell]),
    // Decorations only — opening is via the right-click menu, not click.
    CommentDecorations.configure({ onClickComment: null }),
    Placeholder.configure({ placeholder }),
    Markdown.configure({
      html: false,
      linkify: false,
      breaks: false,
      transformPastedText: true,
      transformCopiedText: false,
    }),
  ];
}
