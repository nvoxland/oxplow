import { useCallback, useEffect, useState } from "react";
import { listExtensions, runLens, subscribeOxplowEvents, type LensRun } from "../api.js";
import type { TabRef } from "../tabs/tabState.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { LensResultView } from "./LensResultView.js";
import { effortRowId, shouldRerunLens, slotMounts } from "./lensModel.js";

const MAX_ROWS = 25;

/**
 * The `effort-review` slot: every lens an extension mounts there, run
 * with this effort's `:effort_id`. Oxplow's own review packet
 * (`oxplow-review`, bundled) arrives this way too. See
 * `.context/extensions.md` → "Slots".
 */
export function EffortReviewSlots({
  effortId,
  streamId,
  onOpenPage,
  h2Style,
}: {
  effortId: string;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
  h2Style: React.CSSProperties;
}) {
  const [runs, setRuns] = useState<{ id: string; run: LensRun | null; error: string | null }[]>([]);
  const rowId = effortRowId(effortId);

  const refresh = useCallback(async () => {
    if (rowId === null) return;
    try {
      const ids = slotMounts(await listExtensions(streamId), "effort-review");
      const next = await Promise.all(
        ids.map(async (id) => {
          try {
            return { id, run: await runLens(id, { effort_id: rowId }, streamId), error: null };
          } catch (e) {
            return { id, run: null, error: e instanceof Error ? e.message : String(e) };
          }
        }),
      );
      setRuns(next);
    } catch {
      setRuns([]);
    }
  }, [rowId, streamId]);

  useEffect(() => {
    void refresh();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((event) => {
      if (!shouldRerunLens({ kind: event.kind, path: (event as { path?: unknown }).path })) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void refresh(), 750);
    });
    return () => {
      if (timer) clearTimeout(timer);
      off();
    };
  }, [refresh]);

  if (runs.length === 0) return null;
  return (
    <>
      {runs.map(({ id, run, error }) => (
        <section key={id} data-testid={`effort-review-${id}`}>
          <h2 style={h2Style}>
            <RouteLink
              to={lensRef(id)}
              onNavigate={() => onOpenPage(lensRef(id))}
              style={{ background: "none", border: "none", padding: 0, font: "inherit", color: "inherit", cursor: "pointer" }}
            >
              {run?.lens.title ?? id}
            </RouteLink>
          </h2>
          {run?.lens.description ? (
            <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", marginBottom: 6 }}>
              {run.lens.description}
            </div>
          ) : null}
          {error ? (
            <div style={{ fontSize: "var(--text-xs)", color: "var(--severity-critical)" }}>{error}</div>
          ) : run ? (
            <LensResultView run={run} onOpenPage={onOpenPage} maxRows={MAX_ROWS} />
          ) : null}
        </section>
      ))}
    </>
  );
}
