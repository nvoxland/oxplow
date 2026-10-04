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
