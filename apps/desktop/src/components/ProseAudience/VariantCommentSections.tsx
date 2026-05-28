import { useMemo } from "react";
import { useCommentsForTarget } from "../Comments/useCommentsForTarget.js";
import { extractHeadingSlugs, sectionSlug } from "./sectionSlug.js";
import { resolveCommentSection } from "./resolveCommentSection.js";

/**
 * When a reader views a non-developer prose variant, a comment's precise
 * quote no longer locates in the text (the prose was rewritten). This
 * panel re-surfaces the page's comments **grouped by the section they
 * were anchored to** (heading slug), so they still appear "in the
 * relevant section" of whichever variant is shown. Comments whose
 * section no longer exists in this variant fall into an "Elsewhere"
 * group. Read-only and display-only — it never re-anchors or writes back
 * (only the developer surface owns that); the comment navigator remains
 * the interactive path.
 */
export function VariantCommentSections({
  targetKind,
  targetId,
  body,
}: {
  targetKind: string;
  targetId: string;
  /** The variant body currently displayed (its headings define the
   *  sections a comment can resolve into). */
  body: string;
}) {
  const { threads } = useCommentsForTarget(targetKind, targetId);

  // Heading text by slug, in document order — so a resolved section can
  // be labeled with its real heading text.
  const headings = useMemo(() => {
    const order: string[] = [];
    const labelBySlug = new Map<string, string>();
    for (const line of body.split("\n")) {
      const m = /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(line);
      if (!m) continue;
      const slug = sectionSlug(m[1]);
      if (slug && !labelBySlug.has(slug)) {
        labelBySlug.set(slug, m[1].trim());
        order.push(slug);
      }
    }
    return { order, labelBySlug };
  }, [body]);

  const grouped = useMemo(() => {
    const headingSlugs = extractHeadingSlugs(body);
    const bySection = new Map<string, typeof threads>();
    const elsewhere: typeof threads = [];
    for (const t of threads) {
      const res = resolveCommentSection(
        { quote: t.comment.quote, sectionAnchor: t.comment.section_anchor },
        body,
        headingSlugs,
      );
      // In a non-developer variant the quote won't match, so resolution
      // is section-or-orphaned; quote-mode (same variant) is handled by
      // the normal highlight layer and skipped here.
      if (res.mode === "section") {
        const list = bySection.get(res.slug) ?? [];
        list.push(t);
        bySection.set(res.slug, list);
      } else if (res.mode === "orphaned") {
        elsewhere.push(t);
      }
    }
    return { bySection, elsewhere };
  }, [threads, body]);

  const total =
    [...grouped.bySection.values()].reduce((n, l) => n + l.length, 0) + grouped.elsewhere.length;
  if (total === 0) return null;

  const renderThread = (t: (typeof threads)[number]) => {
    const first = t.messages[0];
    return (
      <li key={t.comment.id} data-testid={`variant-comment-${t.comment.id}`} style={{ marginBottom: 6 }}>
        {t.comment.quote ? (
          <div
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--text-muted)",
              borderLeft: "2px solid var(--border-subtle)",
              paddingLeft: 6,
              marginBottom: 2,
              fontStyle: "italic",
            }}
          >
            “{t.comment.quote}”
          </div>
        ) : null}
        <div style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>
          {first ? first.body : "(no message)"}
        </div>
      </li>
    );
  };

  return (
    <section
      data-testid="variant-comment-sections"
      style={{
        marginTop: 20,
        paddingTop: 12,
        borderTop: "1px solid var(--border-subtle)",
      }}
    >
      <div
        style={{
          fontSize: 10,
          textTransform: "uppercase",
          letterSpacing: "0.06em",
          color: "var(--text-muted)",
          marginBottom: 8,
        }}
      >
        Comments by section
      </div>
      {headings.order
        .filter((slug) => grouped.bySection.has(slug))
        .map((slug) => (
          <div key={slug} data-testid={`variant-comment-section-${slug}`} style={{ marginBottom: 12 }}>
            <div style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--text-primary)", marginBottom: 4 }}>
              {headings.labelBySlug.get(slug) ?? slug}
            </div>
            <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
              {grouped.bySection.get(slug)!.map(renderThread)}
            </ul>
          </div>
        ))}
      {grouped.elsewhere.length > 0 ? (
        <div data-testid="variant-comment-section-elsewhere" style={{ marginBottom: 12 }}>
          <div style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--text-muted)", marginBottom: 4 }}>
            Elsewhere on this page
          </div>
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {grouped.elsewhere.map(renderThread)}
          </ul>
        </div>
      ) : null}
    </section>
  );
}
