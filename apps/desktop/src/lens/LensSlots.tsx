import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { runLens, type LensRun, type SqlCell } from "../api.js";
import { useExtensions } from "../extensionsStore.js";
import type { TabRef } from "../tabs/tabState.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "./LensResultView.js";
import { foldEmpty, slotRuns } from "./lensModel.js";
import { unionReads, useRerunOnChange } from "./lensRerun.js";

const MAX_ROWS = 25;

/**
 * A slot in a core page: every lens extensions mount at `slot`, run with
 * `params` (the slot's bound values, e.g. `{ effort_id }`). Oxplow's own
 * review packet (bundled `oxplow-bundled`) arrives this way too. See
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
  extension,
}: {
  slot: string;
  /** Only this extension's mounts. */
  extension?: string;
  params: Record<string, SqlCell> | null;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  h2Style?: CSSProperties;
  /** Match the host page's own section headings. */
  h2ClassName?: string;
  variant?: "section" | "strip";
}) {
  const [runs, setRuns] = useState<
    { id: string; params: Record<string, SqlCell>; run: LensRun | null; error: string | null }[]
  >([]);
  const paramsKey = JSON.stringify(params);
  const guard = useRequestGuard();
  const exts = useExtensions();

  const refresh = useCallback(async () => {
    if (params === null || exts === null) return;
    const current = guard.begin();
    try {
      const mounts = slotRuns(exts, slot, params, extension);
      const next = await Promise.all(
        mounts.map(async ({ id, params: lensParams }) => {
          try {
            return { id, params: lensParams, run: await runLens(id, lensParams, streamId), error: null };
          } catch (e) {
            return { id, params: lensParams, run: null, error: e instanceof Error ? e.message : String(e) };
          }
        }),
      );
      if (current()) setRuns(next);
    } catch {
      if (current()) setRuns([]);
    }
    // paramsKey stands in for `params` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [exts, slot, paramsKey, streamId, extension, guard]);

  useEffect(() => {
    // New inputs: drop the old results (another thread's rows) and any
    // answer still in flight for them.
    guard.cancel();
    setRuns([]);
    void refresh();
  }, [refresh, guard]);
  useRerunOnChange(unionReads(runs.map(({ run }) => run?.result.reads)), () => void refresh());

  if (runs.length === 0) return null;
  if (variant === "strip") {
    return (
      <>
        {runs
          .filter(({ run }) => run && run.result.rows.length > 0)
          .map(({ id, run }) => (
            <div key={id} data-testid={`${slot}-${id}`} data-slot={slot} style={stripStyle}>
              <span style={{ textTransform: "uppercase", letterSpacing: "0.04em" }}>{run!.lens.title}</span>
              <LensResultView run={run!} onOpenPage={onOpenPage} streamId={streamId} maxRows={3} compact />
            </div>
          ))}
      </>
    );
  }
  // Sections with nothing to show fold into one closing line (tsk1036).
  const { shown, empty } = foldEmpty(runs);
  return (
    <>
      {shown.map(({ id, params: lensParams, run, error }) => (
        <section key={id} data-testid={`${slot}-${id}`} data-slot={slot} className="lens-section">
          <h2 style={h2Style} className={h2ClassName}>
            <RouteLink
              to={lensRef(id, lensParams)}
              onNavigate={onOpenPage ? () => onOpenPage(lensRef(id, lensParams)) : undefined}
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
      {empty.length > 0 ? (
        <p data-testid={`${slot}-nothing-found`} style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", margin: 0 }}>
          Nothing found:{" "}
          {empty.map(({ id, params: lensParams, run }, i) => (
            <span key={id}>
              {i > 0 ? ", " : null}
              <RouteLink
                to={lensRef(id, lensParams)}
                onNavigate={onOpenPage ? () => onOpenPage(lensRef(id, lensParams)) : undefined}
                style={{ color: "inherit" }}
              >
                {run?.lens.title ?? id}
              </RouteLink>
            </span>
          ))}
          .
        </p>
      ) : null}
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
