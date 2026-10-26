import type { ReactNode } from "react";

/** One number with what it is: a dashboard's or a detail page's figure.
 *  `big` when it is the whole of its place (a number page). */
export function Stat({ label, value, unit, hint, title, big, testId }: { label?: ReactNode; value: ReactNode; unit?: ReactNode; hint?: ReactNode; title?: string; big?: boolean; testId?: string }) {
  return (
    <div className={big ? "ox-stat ox-stat-big" : "ox-stat"} data-testid={testId} title={title}>
      <span className="ox-stat-value">
        {value}
        {unit ? <span className="ox-stat-unit">{unit}</span> : null}
      </span>
      {label ? <span className="ox-stat-label">{label}</span> : null}
      {hint ? <span className="ox-stat-label">{hint}</span> : null}
    </div>
  );
}
