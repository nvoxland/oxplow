import { useEffect, useState } from "react";

import { type MetricSpec, listMetricDefinitions, subscribeOxplowEvents } from "../api.js";
import { cardLinkButton } from "../components/Card.js";
import { metricRef, recordedMetricsRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricsExplorer } from "./MetricsExplorer.js";

/**
 * Metrics Explorer — the marquee *observe* surface over the unified metric
 * substrate (epic tsk213): multi-measure charts over time, group-by a
 * conformed dimension, target bands, saved/built-in presets, effort bands.
 *
 * Split from the old combined Metrics page (tsk283) so each surface has one
 * job: this page owns the charts, `RecordedMetricsPage` owns the table, and
 * `MetricDetailPage` owns the per-metric drill-in. A measure's title links into
 * the detail page (`metricRef`); the header cross-links to the other two.
 */
export function MetricsExplorerPage({
  initialPreset,
  initialEffort,
  onOpenPage,
}: {
  initialPreset?: string;
  /** Deep-link: scope the chart window to this effort. */
  initialEffort?: { effortId: string; start: string; end: string | null };
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  const [defs, setDefs] = useState<MetricSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const nav = useOptionalPageNavigation();

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then((d) => {
        if (!cancelled) {
          setDefs(d);
          setLoading(false);
        }
      });
    };
    refresh();
    // Trailing-debounce the reload: the OTLP token ingest emits
    // metricSamplesChanged on every agent turn, and the Explorer's own
    // `MetricsExplorer` refetches each selected measure's series off these defs
    // (tsk91 — see RecordedMetricsPage / the tsk75 EffortMetricsBlock fix).
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((e) => {
      if (e.kind !== "metricSamplesChanged") return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(refresh, 2_500);
    });
    return () => {
      cancelled = true;
      off();
      if (timer) clearTimeout(timer);
    };
  }, []);

  const openDetail = (def: MetricSpec) => {
    const ref = metricRef(def.key);
    if (nav) nav.navigate(ref);
    else onOpenPage?.(ref);
  };

  return (
    <Page
      testId="page-metrics"
      title="Metrics"
      actions={
        <div style={{ display: "flex", gap: 12 }}>
          <RouteLink
            to={recordedMetricsRef()}
            onNavigate={onOpenPage}
            style={cardLinkButton}
            testId="metrics-recorded-link"
          >
            Recorded metrics →
          </RouteLink>
        </div>
      }
    >
      <div style={{ padding: "16px 20px", maxWidth: 1000 }}>
        {loading ? (
          <div style={{ opacity: 0.6 }}>Loading…</div>
        ) : defs.length === 0 ? (
          <div style={{ opacity: 0.6, lineHeight: 1.6 }}>
            No metrics recorded yet. Run tests, coverage, or static analysis —
            oxplow records them into the substrate automatically. Custom metrics
            can be declared in <code>.oxplow/project.yaml</code>.
          </div>
        ) : (
          <MetricsExplorer
            defs={defs}
            onOpenDetail={openDetail}
            initialPreset={initialPreset}
            initialScope={
              initialEffort && !Number.isNaN(Date.parse(initialEffort.start))
                ? {
                    start: Date.parse(initialEffort.start),
                    // Open effort (no end) → scope through "now".
                    end: initialEffort.end ? Date.parse(initialEffort.end) : Date.now(),
                  }
                : undefined
            }
          />
        )}
      </div>
    </Page>
  );
}
