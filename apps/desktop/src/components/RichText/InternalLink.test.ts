import { expect, test } from "bun:test";

import { InternalLink } from "./InternalLink.js";

// tsk974: Tiptap registers each of Link's `protocols` with linkify when an
// editor is made, and linkify throws on a scheme it can't tokenize
// (`work_item` has an underscore) — so every rich-text field threw. Our
// schemes are allowed through `isAllowedUri` instead.

const options = InternalLink.options as {
  protocols: Array<string | { scheme: string }>;
  isAllowedUri: (url: string, ctx: { defaultValidate: (u: string) => boolean; protocols: unknown[] }) => boolean;
};

test("every scheme the editor registers with linkify is one linkify accepts", () => {
  const schemes = options.protocols.map((p) => (typeof p === "string" ? p : p.scheme));
  for (const scheme of schemes) expect(scheme).toMatch(/^[0-9a-z]+(-[0-9a-z]+)*$/);
});

test("internal links are allowed and a script url isn't", () => {
  const ctx = { defaultValidate: (u: string) => /^(https?|mailto):/i.test(u), protocols: [] };
  for (const url of ["work_item:oxplow:tsk1", "file:src/a.ts", "dir:src", "commit:abc123", "https://example.com"]) {
    expect(options.isAllowedUri(url, ctx)).toBe(true);
  }
  expect(options.isAllowedUri("javascript:alert(1)", ctx)).toBe(false);
});

// tsk1007: an invalid wikilink is an `oxplow-invalid:` link (the preprocess
// keeps what it can't resolve as one), kept through the editor so it saves
// back as `[[…]]`; and a link Tiptap hands over without an href doesn't
// throw.
test("an invalid wikilink's link is kept, and a missing href doesn't throw", () => {
  const ctx = { defaultValidate: (u: string | null) => !u || /^(https?|mailto):/i.test(u), protocols: [] };
  expect(options.isAllowedUri("oxplow-invalid:%2313", ctx)).toBe(true);
  expect(() => options.isAllowedUri(null as unknown as string, ctx)).not.toThrow();
  const storage = (InternalLink.config.addStorage as () => {
    markdown: { parse: { setup(md: { validateLink: (url: string) => boolean }): void } };
  }).call({ parent: undefined });
  const md = { validateLink: (_url: string) => false };
  storage.markdown.parse.setup(md);
  expect(md.validateLink("oxplow-invalid:%2313")).toBe(true);
  expect(md.validateLink("javascript:alert(1)")).toBe(false);
});
