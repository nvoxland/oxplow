import type { HTMLAttributes, ReactNode } from "react";

export interface Fact {
  key: string;
  label: ReactNode;
  value: ReactNode;
}

/** Labels beside their values: one record's facts (a metric's
 *  definition, a page's `detail`). */
export function Facts({ facts, testId, ...rest }: HTMLAttributes<HTMLElement> & { facts: readonly Fact[]; testId?: string }) {
  return (
    <dl className="ox-facts" data-testid={testId} {...rest}>
      {facts.map((f) => (
        <div key={f.key} className="ox-fact">
          <dt>{f.label}</dt>
          <dd>{f.value}</dd>
        </div>
      ))}
    </dl>
  );
}
