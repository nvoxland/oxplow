import type { ReactNode } from "react";

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
  children?: ReactNode;
}

/** A part of a page: a heading over a rule, its body under it. No box —
 *  pages read as documents. */
export function Section({ id, title, description, actions, source, testId, children }: SectionProps) {
  return (
    <section className="ox-section" data-section={id} data-testid={testId}>
      <div className="ox-section-head">
        <h2 className="ox-section-title">{title}</h2>
        {source ? (
          <span className="ox-badge" data-source={source} title={`Added by the ${source} extension`}>
            {source}
          </span>
        ) : null}
        {actions ? <ActionButtons actions={actions} /> : null}
      </div>
      {description ? <p className="ox-section-desc">{description}</p> : null}
      {children}
    </section>
  );
}
