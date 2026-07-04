import { fileRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricsCatalog } from "./MetricsCatalog.js";

/**
 * Metrics Catalog — the *configure* surface for the metric substrate
 * (epic tsk213), split off from the *observe* Metrics page (tsk282).
 *
 * Browse the available catalog (built-in ∪ global ∪ project), enable/disable
 * a metric (writes a `use:` into `.oxplow/project.yaml`), inline-edit target/trigger,
 * and scaffold new gauges. The Metrics page stays read-only (Explorer +
 * Recorded metrics + detail); this page owns every write.
 */
export function MetricsCatalogPage({
  onOpenPage,
}: {
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  return (
    <Page testId="page-metrics-catalog" title="Metrics Catalog">
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
