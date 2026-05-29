import { useEffect, useState } from "react";
import {
  type EffortObservation,
  listEffortObservations,
  subscribeOxplowEvents,
} from "../api.js";
import { FullCoverageView } from "../components/EffortObservations.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";

export function EffortCoveragePage({
  effortId,
  onOpenFile,
}: {
  effortId: string;
  onOpenFile?: (path: string) => void;
}) {
  usePageTitle("Coverage & tests");
  const [obs, setObs] = useState<EffortObservation[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void listEffortObservations(effortId).then((rows) => {
        if (!cancelled) setObs(rows);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "effortObservationsChanged") return;
      load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [effortId]);

  return (
    <Page showNavBar showHeader>
      <div style={{ padding: "24px 32px", maxWidth: 800 }}>
        {obs.length === 0 ? (
          <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
            No observations recorded for this effort.
          </span>
        ) : (
          <FullCoverageView effortId={effortId} obs={obs} onOpenFile={onOpenFile} />
        )}
      </div>
    </Page>
  );
}
