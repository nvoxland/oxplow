import type { CSSProperties } from "react";

/**
 * The expand / collapse affordance for a slide-out glyph strip (see
 * {@link ../components/useSlideoutStrip.ts}). Pinned to the BOTTOM of the
 * strip, and mirrored at the bottom of the open panel.
 *
 * Two non-obvious requirements it exists to enforce, both learned in
 * tsk269 / tsk270:
 *
 * - **Pinned outside the scroll container.** Clicking the strip's dead
 *   space also expands, but that can't be the only route: a list long
 *   enough to scroll has no dead space left, which is exactly when the
 *   panel is most wanted. The chevron is always reachable.
 * - **Left-aligned inside a strip-width box, not centered in its own
 *   container.** In the strip that reads as centered (the container *is*
 *   strip-width); in the much wider panel it keeps the control at the same
 *   x-position, so it doesn't jump sideways as the panel opens and closes.
 *   Same lock-step rule the glyph rows follow for their y-positions.
 */
export function SlideoutChevron({
  direction,
  stripWidth,
  testId,
  title,
  onClick,
}: {
  direction: "expand" | "collapse";
  stripWidth: number;
  testId: string;
  title: string;
  onClick(): void;
}) {
  return (
    <div style={containerStyle}>
      <div style={{ width: stripWidth, display: "flex", justifyContent: "center" }}>
        <button
          type="button"
          data-testid={testId}
          onClick={onClick}
          title={title}
          aria-label={title}
          aria-expanded={direction === "collapse"}
          style={{ ...buttonStyle, width: stripWidth - 10 }}
        >
          {direction === "expand" ? "›" : "‹"}
        </button>
      </div>
    </div>
  );
}

const containerStyle: CSSProperties = {
  flexShrink: 0,
  borderTop: "1px solid var(--border-subtle)",
  padding: "4px 0",
  display: "flex",
  justifyContent: "flex-start",
};

const buttonStyle: CSSProperties = {
  height: 24,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "transparent",
  color: "var(--text-secondary)",
  border: "1px solid transparent",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
  lineHeight: 1,
};
