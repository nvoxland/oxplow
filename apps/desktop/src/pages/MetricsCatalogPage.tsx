import { fileRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricsCatalog } from "./MetricsCatalog.js";

/**
 * Metric Settings — the *configure* surface for the metric substrate
 * (epic tsk213), split off from the *observe* Metrics page (tsk282).
 *
 * Browse the available catalog (built-in ∪ global ∪ project), enable/disable
 * a metric (writes a `use:` into `.oxplow/project.yaml`), inline-edit target/trigger,
 * and scaffold new gauges. The Metrics page stays read-only (Explorer +
 * Recorded metrics + detail); this page owns every write.
 *
 * Titled "Metric Settings" (tsk80): "Catalog" read like a browsable index —
 * which is what Recorded Metrics is. The page kind / tab id / `metricsCatalogRef`
 * keep the `metrics-catalog` slug so existing refs and bookmarks still resolve.
 */
export function MetricsCatalogPage({
  onOpenPage,
}: {
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  return (
    <Page testId="page-metrics-catalog" title="Metric Settings">
      <div
        style={{
          padding: "16px 20px",
          display: "flex",
          flexDirection: "column",
          gap: 16,
          maxWidth: 1000,
        }}
      >
        <MetricsCatalog
          onOpenScript={onOpenPage ? (path) => onOpenPage(fileRef(path)) : undefined}
          onOpenPage={onOpenPage}
        />
      </div>
    </Page>
  );
}
