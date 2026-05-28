// How a comment resolves against the prose variant currently displayed.
//
// A comment's precise `quote` only locates in the variant it was
// authored on. When a different variant is shown the quote won't match,
// so we fall back to the stored `section_anchor` (a heading slug) and
// re-display the comment under the matching heading. With neither, the
// comment is orphaned for this variant (still reachable via the
// navigator, just not anchored in-text).

export type CommentSectionResolution =
  | { mode: "quote" }
  | { mode: "section"; slug: string }
  | { mode: "orphaned" };

export interface ResolvableComment {
  quote: string;
  /** Stored heading-slug anchor, or null when absent. */
  sectionAnchor?: string | null;
}

/**
 * Resolve `comment` against the body + headings of the variant being
 * viewed. Precedence: exact quote match (precise highlight) → section
 * anchor present among the variant's headings (section-level marker) →
 * orphaned.
 *
 * This never writes back — only the authoring (developer) surface may
 * persist `orphaned` / re-anchor. Cross-variant resolution is
 * display-only.
 */
export function resolveCommentSection(
  comment: ResolvableComment,
  displayedBody: string,
  headingSlugs: string[],
): CommentSectionResolution {
  if (comment.quote && displayedBody.includes(comment.quote)) {
    return { mode: "quote" };
  }
  const anchor = comment.sectionAnchor;
  if (anchor && headingSlugs.includes(anchor)) {
    return { mode: "section", slug: anchor };
  }
  return { mode: "orphaned" };
}
