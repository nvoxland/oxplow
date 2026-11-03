/// What a dock panel's controls look like. A panel is a popover (the
/// elevated tier, `.context/theming.md`), so a secondary button's
/// transparent face would vanish on it: a dock button wears the card tier
/// and the primary text, so it reads as a button at a glance.

import type { CSSProperties } from "react";

export const dockButtonStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  fontWeight: "var(--weight-medium)",
};
