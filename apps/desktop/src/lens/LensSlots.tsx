import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { listExtensions, runLens, subscribeOxplowEvents, type LensRun, type SqlCell } from "../api.js";
import type { TabRef } from "../tabs/tabState.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { LensResultView } from "./LensResultView.js";
import { shouldRerunLens, slotMounts } from "./lensModel.js";

const MAX_ROWS = 25;

/**
 * A slot in a core page: every lens extensions mount at `slot`, run with
 * `params` (the slot's bound values, e.g. `{ effort_id }`). Oxplow's own
 * review packet (`oxplow-review`, bundled) arrives this way too. See
 * `.context/extensions.md` → "Slots".
 *
 * `section` (default) gives each lens a heading; `strip` is a compact
 * one-line form for narrow panes and hides lenses with no rows.
 */
export function LensSlots({
  slot,
  params,
  streamId,
  onOpenPage,
  h2Style,
  h2ClassName,
  variant = "section",
}: {
  slot: string;
  params: Record<string, SqlCell> | null;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  h2Style?: CSSProperties;
  /** Match the host page's own section headings. */
  h2ClassName?: string;
  variant?: "section" | "strip";
}) {
  const [runs, setRuns] = useState<{ id: string; run: LensRun | null; error: string | null }[]>([]);
  const paramsKey = JSON.stringify(params);

  const refresh = useCallback(async () => {
    if (params === null) return;
    try {
      const ids = slotMounts(await listExtensions(streamId), slot);
      const next = await Promise.all(
        ids.map(async (id) => {
          try {
            return { id, run: await runLens(id, params, streamId), error: null };
          } catch (e) {
            return { id, run: null, error: e instanceof Error ? e.message : String(e) };
          }
        }),
      );
      setRuns(next);
    } catch {
      setRuns([]);
    }
    // paramsKey stands in for `params` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [slot, paramsKey, streamId]);

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
  if (variant === "strip") {
    return (
      <>
        {runs
          .filter(({ run }) => run && run.result.rows.length > 0)
          .map(({ id, run }) => (
            <div key={id} data-testid={`${slot}-${id}`} style={stripStyle}>
              <span style={{ textTransform: "uppercase", letterSpacing: "0.04em" }}>{run!.lens.title}</span>
              <LensResultView run={run!} onOpenPage={onOpenPage} streamId={streamId} maxRows={3} compact />
            </div>
          ))}
      </>
    );
  }
  return (
    <>
      {runs.map(({ id, run, error }) => (
        <section key={id} data-testid={`${slot}-${id}`}>
          <h2 style={h2Style} className={h2ClassName}>
            <RouteLink
              to={lensRef(id)}
              onNavigate={onOpenPage ? () => onOpenPage(lensRef(id)) : undefined}
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
            <LensResultView run={run} onOpenPage={onOpenPage} streamId={streamId} maxRows={MAX_ROWS} />
          ) : null}
        </section>
      ))}
    </>
  );
}

const stripStyle: CSSProperties = {
  display: "flex",
  alignItems: "baseline",
  gap: 6,
  padding: "4px 8px",
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  borderBottom: "1px solid var(--border-subtle)",
};
