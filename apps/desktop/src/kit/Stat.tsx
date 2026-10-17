import type { ReactNode } from "react";

/** One number with what it is: a dashboard's or a detail page's figure. */
export function Stat({ label, value, unit, hint }: { label: ReactNode; value: ReactNode; unit?: ReactNode; hint?: ReactNode }) {
  return (
    <div className="ox-stat">
      <span className="ox-stat-value">
        {value}
        {unit ? <span className="ox-stat-unit">{unit}</span> : null}
      </span>
      <span className="ox-stat-label">{label}</span>
      {hint ? <span className="ox-stat-label">{hint}</span> : null}
    </div>
  );
}
