import type { ReactNode } from "react";

import type { PageAction } from "./actions.js";

export interface RailGroupProps {
  title: string;
  /** One small action beside the heading (Filters → Reset). */
  action?: PageAction;
  testId?: string;
  children: ReactNode;
}

/** One group of a page's rail under its small heading. A rail stacks its
 *  groups in a fixed order (Filters, Selection, About this page), so
 *  nothing moves as values change. */
export function RailGroup({ title, action, testId, children }: RailGroupProps) {
  return (
    <section className="ox-rail-group" data-testid={testId} aria-label={title}>
      <h3 className="ox-rail-group-head">
        <span>{title}</span>
        {action ? (
          <button type="button" className="ox-link" data-testid={`action-${action.id}`} title={action.title} onClick={() => void action.run()}>
            {action.label}
          </button>
        ) : null}
      </h3>
      {children}
    </section>
  );
}
