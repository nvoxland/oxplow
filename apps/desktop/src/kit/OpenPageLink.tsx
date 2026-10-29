import { SquareArrowRightEnter } from "lucide-react";

import { RouteLink } from "../tabs/RouteLink.js";
import type { Origin } from "../tabs/session.js";
import type { TabRef } from "../tabs/tabState.js";

export interface OpenPageLinkProps {
  /** The page it opens: a tab or a ref string. */
  to: TabRef | string;
  /** What it opens, in words (the heading's text): "Open <name>". */
  name: string;
  onNavigate?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
  origin?: Origin;
}

/** The arrow after a heading whose thing has a page of its own: one icon,
 *  used for nothing else, so it reads as "open this thing's page". The
 *  heading itself is never a link (`.context/usability.md` "Pages are
 *  informational"). */
export function OpenPageLink({ to, name, onNavigate, origin }: OpenPageLinkProps) {
  return (
    <RouteLink
      to={to}
      testId="open-page-link"
      className="ox-open-page"
      title={`Open ${name}`}
      ariaLabel={`Open ${name}`}
      onNavigate={onNavigate}
      origin={origin}
    >
      <SquareArrowRightEnter size={16} aria-hidden="true" />
    </RouteLink>
  );
}
