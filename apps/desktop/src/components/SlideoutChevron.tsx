import type { CSSProperties } from "react";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";

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
 *
 * It shows the standard open / close-sidebar icon, outlined and with a
 * hover state (`.oxplow-slideout-toggle` in `index.html`), so it reads as
 * "show / hide this panel" — a bare `›` read as decoration.
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
          className="oxplow-slideout-toggle"
          style={{ ...buttonStyle, width: stripWidth - 10 }}
        >
          {direction === "expand" ? <PanelLeftOpen size={16} aria-hidden /> : <PanelLeftClose size={16} aria-hidden />}
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

// Background, color and border come from `.oxplow-slideout-toggle` so
// its `:hover` can change them (an inline style would win over it).
const buttonStyle: CSSProperties = {
  height: 26,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  borderRadius: 6,
  cursor: "pointer",
  padding: 0,
};
