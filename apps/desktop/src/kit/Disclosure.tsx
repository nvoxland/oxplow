import { ChevronDown, ChevronRight } from "lucide-react";

/** The glyph every disclosure shares: a row's expand, a group's fold, a
 *  tree's branch, a section's toggle. One size, so it is seen; the
 *  button around it says what it opens (`aria-expanded`, `aria-label`). */
export function DisclosureGlyph({ open }: { open: boolean }) {
  return open ? <ChevronDown size={16} aria-hidden /> : <ChevronRight size={16} aria-hidden />;
}
