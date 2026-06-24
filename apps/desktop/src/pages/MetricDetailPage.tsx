import { useEffect, useState } from "react";

import { type MetricDefinition, listMetricDefinitions, subscribeOxplowEvents } from "../api.js";
import { Card } from "../components/Card.js";
import { recordedMetricsRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricDetail } from "./MetricDetail.js";

/**
 * Metric Detail — one metric's per-kind drill-in (trend + Δ-vs-first + the
 * kind-specific detail: findings table / test tree / coverage lines / top
 * subjects). Its own page (tsk283), routed by `metricRef(key, effort)`, so the
 * Explorer and Recorded Metrics pages navigate into it instead of toggling an
 * inline overlay. When opened from the task-page metrics panel the `effort`
 * payload scopes an "In this effort" before→after callout above the trend.
 */
export function MetricDetailPage({
  metricKey,
  effort,
  onOpenPage,
}: {
  metricKey?: string;
  effort?: { effortId: string; start: string; end: string | null };
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  const [def, setDef] = useState<MetricDefinition | null>(null);
  const [loading, setLoading] = useState(true);
  const nav = useOptionalPageNavigation();
  usePageTitle(def?.title ?? "Metric");

  useEffect(() => {
    if (!metricKey) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then((defs) => {
        if (cancelled) return;
        setDef(defs.find((d) => d.key === metricKey) ?? null);
        setLoading(false);
      });
    };
    refresh();
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "metricSamplesChanged") refresh();
    });
    return () => {
      cancelled = true;
      off();
    };
  }, [metricKey]);

  const goBack = () => {
    const recorded = recordedMetricsRef();
    if (nav?.canGoBack) nav.goBack();
    else if (nav) nav.navigate(recorded);
    else onOpenPage?.(recorded);
  };

  return (
    <Page testId="page-metric-detail" title={def?.title ?? "Metric"}>
      <div style={{ padding: "16px 20px", maxWidth: 1000 }}>
        <Card testId="metric-detail-card" title="Metric detail">
          {loading ? (
            <div style={{ opacity: 0.6 }}>Loading…</div>
          ) : def ? (
            <MetricDetail def={def} effort={effort} onBack={goBack} />
          ) : (
            <div style={{ opacity: 0.6 }}>Metric not found.</div>
          )}
        </Card>
      </div>
    </Page>
  );
}
