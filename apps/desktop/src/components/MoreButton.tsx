import { Menu as MenuGlyph } from "lucide-react";

import { useCoarsePointer } from "../coarsePointer.js";

/**
 * A finger's way to a menu it would otherwise find only by right-click or a
 * long press: a ☰ that opens the same menu, below itself. Drawn only for a
 * coarse pointer (a mouse has the right-click, and a desktop row keeps its
 * space). Its tap is its own, not its row's.
 */
export function MoreButton({
  onOpen,
  testId,
  label = "Actions",
}: {
  /** Open the menu at this point (the button's bottom left). */
  onOpen(at: { x: number; y: number }): void;
  testId?: string;
  label?: string;
}) {
  const coarse = useCoarsePointer();
  if (!coarse) return null;
  return (
    <button
      type="button"
      className="ox-icon-button ox-row-menu"
      data-testid={testId}
      title={label}
      aria-label={label}
      aria-haspopup="menu"
      onClick={(e) => {
        e.stopPropagation();
        e.preventDefault();
        const rect = e.currentTarget.getBoundingClientRect();
        onOpen({ x: rect.left, y: rect.bottom + 2 });
      }}
    >
      <MenuGlyph size={14} aria-hidden />
    </button>
  );
}
