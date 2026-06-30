import Link from "@tiptap/extension-link";

/**
 * Tiptap Link mark extended to permit our internal URL schemes
 * (`file:`, `dir:`, `gitcommit:`, `task:`). Tiptap's default `Link` allowlists
 * http/https/mailto/ftp; without this extension our schemes either
 * get stripped on parse or fail the click-validation in the standard
 * link plugin.
 *
 * Click handling for these schemes still happens at the React layer
 * (the `RichTextField` wrapper's `onClick` / `onAuxClick` intercepts
 * clicks on `<a>` descendants and routes them through
 * `useOptionalPageNavigation`, mirroring `MarkdownView`), so the mark
 * itself only needs to *preserve* the URL through parse/serialize.
 *
 * Round-trip note: markdown-it (used by tiptap-markdown to parse the
 * editor's markdown input) ships a `validateLink` that hard-rejects
 * `file:` and `data:` URLs out of XSS caution. Without the
 * `markdown.parse.setup` hook below, our `[label](file:path)`
 * preprocessed wikilinks would never become Link marks — they'd
 * round-trip as escaped literal text (`\[[path|path\]]` on disk).
 * The hook monkey-patches `md.validateLink` to additionally permit
 * the schemes we own. tiptap-markdown discovers this storage spec by
 * name match against its internal `link` extension and invokes
 * `setup(md)` at parse time.
 */
const INTERNAL_PROTOCOL_RE = /^(file|dir|gitcommit|task):/i;

export const InternalLink = Link.extend({
  // Allow our schemes through the URL sanitizer.
  addOptions() {
    return {
      ...this.parent?.(),
      openOnClick: false,
      autolink: false,
      protocols: ["file", "dir", "gitcommit", "task"],
    };
  },
  addStorage() {
    return {
      ...(this.parent?.() ?? {}),
      markdown: {
        parse: {
          setup(md: { validateLink: (url: string) => boolean }) {
            const original = md.validateLink.bind(md);
            md.validateLink = (url: string) => {
              if (INTERNAL_PROTOCOL_RE.test(url.trim())) return true;
              return original(url);
            };
          },
        },
      },
    };
  },
});
