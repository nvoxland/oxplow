import type { SqlCell } from "../api.js";
import type { LensParam } from "../tauri-bridge/generated/bindings.js";
import { ParamRow } from "./ParamControls.js";

/** The quick picks of a metric's range. */
export const METRIC_RANGE_PRESETS = ["1d", "2d", "3d", "7d", "30d"];

/** The filters a metrics page offers; a field left out is not drawn. */
export interface MetricFiltersProps {
  /** A range value: a preset key, `all`, or `a..b` (`kit/rangePresets.ts`). */
  range?: string;
  /** The quick picks of `range`; the metric ones by default. */
  presets?: string[];
  /** A branch name; "" is every branch. */
  branch?: string;
  /** The branches the metric was recorded on. */
  branches?: readonly string[];
  /** A dimension key; "" is none. */
  slice?: string;
  /** The dimensions a metric can be sliced by; none hides the filter. */
  dimensions?: readonly { key: string; label: string }[];
  onApply(name: "range" | "branch" | "slice", value: string): void;
}

/**
 * The range, branch and slice filters of the Metrics, Metric and Custom
 * Dashboard pages, as the rows a lens page's rail has (`ParamRow`) over
 * params made here — so a metric is filtered the way every other page is,
 * and the range is the lens `date_range`'s vocabulary.
 */
export function MetricFilters({
  range,
  presets = METRIC_RANGE_PRESETS,
  branch,
  branches = [],
  slice,
  dimensions = [],
  onApply,
}: MetricFiltersProps) {
  const rows: { param: LensParam; value: SqlCell }[] = [];
  if (range !== undefined) rows.push({ param: { name: "range", type: "date_range", default: "7d", presets }, value: range });
  if (branch !== undefined) {
    rows.push({
      param: {
        name: "branch",
        type: "choice",
        default: "",
        options: [{ value: "", label: "All branches" }, ...branches.map((b) => ({ value: b, label: b }))],
      },
      value: branch,
    });
  }
  if (slice !== undefined && dimensions.length > 0) {
    rows.push({
      param: {
        name: "slice",
        label: "Slice by",
        type: "choice",
        default: "",
        options: [{ value: "", label: "No slice" }, ...dimensions.map((d) => ({ value: d.key, label: d.label }))],
      },
      value: slice,
    });
  }
  return (
    <div className="ox-params" data-testid="metric-filters">
      {rows.map(({ param, value }) => (
        <ParamRow
          key={param.name}
          param={param}
          value={value}
          onApply={(name, next) => onApply(name as "range" | "branch" | "slice", next === null ? "" : String(next))}
        />
      ))}
    </div>
  );
}
