import type { ReactNode } from "react";

import { useSectionCollapse } from "../components/CollapsibleSections.js";
import { ActionButtons } from "./ActionButtons.js";
import type { PageAction } from "./actions.js";

export interface SectionProps {
  /** The section's stable id: what an extension's mount names to go
   *  beside it (`at: { after: <id> }`). */
  id?: string;
  title: ReactNode;
  description?: ReactNode;
  actions?: readonly PageAction[];
  /** The extension it comes from, when it isn't the page's own: said by
   *  the heading, so added content is never mistaken for the page's. */
  source?: string;
  testId?: string;
  /** The heading toggles the body open and closed, remembered per page
   *  through the page's `CollapsibleSections` provider (and its Expand all /
   *  Collapse all). Needs an `id`; outside a provider the section is plain. */
  collapsible?: boolean;
  /** A collapsible section starts closed (until the person opens it). */
  defaultCollapsed?: boolean;
  children?: ReactNode;
}

/** A part of a page: a heading over a rule, its body under it. No box —
 *  pages read as documents. */
export function Section({
  id,
  title,
  description,
  actions,
  source,
  testId,
  collapsible = false,
  defaultCollapsed = false,
  children,
}: SectionProps) {
  const collapse = useSectionCollapse(id, collapsible, defaultCollapsed);
  const open = collapse ? collapse.expanded : true;
  return (
    <section
      className="ox-section"
      data-section={id}
      data-testid={testId ?? (collapse ? `${collapse.prefix}-group-${id}` : undefined)}
    >
      <div className="ox-section-head">
        <h2 className="ox-section-title">
          {collapse ? (
            // The toggle sits INSIDE the heading: it stays a heading for the
            // outline, and a real button gets the keyboard for free.
            <button
              type="button"
              className="ox-section-toggle"
              data-testid={`${collapse.prefix}-section-toggle-${id}`}
              aria-expanded={collapse.expanded}
              onClick={collapse.toggle}
            >
              <span aria-hidden className="ox-section-chevron">
                {collapse.expanded ? "▾" : "▸"}
              </span>
              <span>{title}</span>
            </button>
          ) : (
            title
          )}
        </h2>
        {source ? (
          <span className="ox-badge" data-source={source} title={`Added by the ${source} extension`}>
            {source}
          </span>
        ) : null}
        {actions ? <ActionButtons actions={actions} /> : null}
      </div>
      {open ? (
        collapse ? (
          <div data-testid={`${collapse.prefix}-section-body-${id}`}>
            {description ? <p className="ox-section-desc">{description}</p> : null}
            {children}
          </div>
        ) : (
          <>
            {description ? <p className="ox-section-desc">{description}</p> : null}
            {children}
          </>
        )
      ) : null}
    </section>
  );
}
