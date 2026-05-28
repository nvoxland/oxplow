// Heading-slug derivation for comment section anchoring across prose
// variants. MUST stay in lockstep with the Rust `heading_slug`
// (crates/oxplow-domain/src/prose.rs): lowercase, keep alphanumerics,
// collapse whitespace / `-` / `_` runs to a single hyphen, drop all
// other punctuation, trim trailing hyphens. The two are tested against
// the same fixtures.

export function sectionSlug(text: string): string {
  let slug = "";
  let prevHyphen = false;
  for (const ch of text) {
    if (/[\p{L}\p{N}]/u.test(ch)) {
      slug += ch.toLowerCase();
      prevHyphen = false;
    } else if (/\s/.test(ch) || ch === "-" || ch === "_") {
      if (!prevHyphen && slug.length > 0) {
        slug += "-";
        prevHyphen = true;
      }
    }
    // all other punctuation is dropped
  }
  while (slug.endsWith("-")) slug = slug.slice(0, -1);
  return slug;
}

/** Slugs of every ATX heading (`#`..`######`) in a markdown body, in
 *  document order. Used to test whether a comment's stored section
 *  anchor resolves in the variant currently being viewed. */
export function extractHeadingSlugs(markdown: string): string[] {
  const out: string[] = [];
  for (const line of markdown.split("\n")) {
    const m = /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(line);
    if (m) {
      const slug = sectionSlug(m[1]);
      if (slug) out.push(slug);
    }
  }
  return out;
}

/** The heading slug of the section a `quote` sits inside within `body`
 *  — the nearest ATX heading preceding the quote's first occurrence.
 *  Returns `null` for a missing/empty quote or a quote that precedes
 *  every heading (whole-target). Captured at comment-create time so the
 *  comment can re-display under the matching heading of another variant. */
export function sectionAnchorForQuote(body: string, quote: string): string | null {
  if (!quote) return null;
  const at = body.indexOf(quote);
  if (at < 0) return null;
  const before = body.slice(0, at);
  let found: string | null = null;
  for (const line of before.split("\n")) {
    const m = /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(line);
    if (m) {
      const slug = sectionSlug(m[1]);
      if (slug) found = slug;
    }
  }
  return found;
}
