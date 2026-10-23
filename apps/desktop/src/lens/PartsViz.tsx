import type { PickedRow } from "./LensRail.js";
import type { ReactNode } from "react";

import type { LensRun, SqlCell } from "../api.js";
import type { LensPart } from "../tauri-bridge/generated/bindings.js";
import { Replaceable } from "../components/Replaceable.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import { Loading } from "../kit/Loading.js";
import { Section } from "../kit/Section.js";
import { Tile, TileGrid, TileSlot } from "../kit/Tiles.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { LensResultView } from "./LensResultView.js";
import { foldEmpty } from "./lensModel.js";
import { useLensRuns, type PartRun } from "./useLensRuns.js";

/** Rows a dashboard tile shows before "open the lens for the rest". */
const TILE_MAX_ROWS = 8;

interface PartsVizProps {
  parts: readonly LensPart[];
  /** The page's params: what a part's row click may set. */
  params: Record<string, SqlCell>;
  /** What its parts are run with (`partParams`: its row under `params`);
   *  `params` when it has no row. */
  given?: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  /** Set one of the page's params, from a part's row click. */
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
}

/** The lens parts of `parts`, run. */
function usePartRuns({ parts, params, given, streamId }: PartsVizProps) {
  return useLensRuns(
    parts.flatMap((p) => (p.lens ? [{ key: p.id, lensId: p.lens }] : [])),
    given ?? params,
    streamId,
  );
}

/** A part's heading: its own title, else its lens's, linked to the lens. */
function PartTitle({ part, run, onOpenPage }: { part: LensPart; run: LensRun | null; onOpenPage?(ref: TabRef): void }) {
  const text = part.title ?? run?.lens.title ?? part.lens ?? part.id;
  if (!part.lens) return <>{text}</>;
  const to = lensRef(part.lens);
  return (
    <RouteLink to={to} onNavigate={onOpenPage ? () => onOpenPage(to) : undefined} style={titleLinkStyle}>
      {text}
    </RouteLink>
  );
}

/** A part's body: its prose, its result, its error, or that it's loading. */
function PartBody({
  part,
  run,
  maxRows,
  streamId,
  onOpenPage,
  params,
  onSetParam,
}: {
  part: LensPart;
  run: PartRun | undefined;
  /** A tile's cap; a document section (the page itself) shows every row. */
  maxRows?: number;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  params: Record<string, SqlCell>;
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
}): ReactNode {
  if (part.markdown !== null) return <Markdown value={part.markdown} />;
  if (!run) return <Loading what={part.title ?? "this part"} />;
  if (run.error) return <p className="ox-error">{run.error}</p>;
  if (!run.run) return null;
  // Its rows pick only a param the page holds: one another extension's
  // part selects that the page doesn't declare stays plain.
  const selects = run.run.lens.columns.find((c) => c.select)?.select;
  const set = selects && selects in params ? onSetParam : undefined;
  const own = <LensResultView run={run.run} onOpenPage={onOpenPage} streamId={streamId} maxRows={maxRows} toolbar={false} partButtons onSetParam={set} />;
  // A part standing in for a core component: its lens is oxplow's own,
  // shown unless an extension replaces the target; the replacement is given
  // the part's resolved params (the contract the loader checked it declares).
  if (!part.replaceable) return own;
  return <Replaceable target={part.replaceable} props={run.run.params} streamId={streamId} onOpenPage={onOpenPage} fallback={own} />;
}

/**
 * `document`: its parts as sections, wiki style — a heading over a rule,
 * no boxes. What other extensions mounted is among them (placed by the
 * catalog) and says whose it is. Sections with nothing to show fold into
 * one closing line (tsk1036).
 */
export function DocumentViz(props: PartsVizProps) {
  const runs = usePartRuns(props);
  const { parts, streamId, onOpenPage, params, onSetParam } = props;
  // A part that needs what isn't active (a link, where the work list keeps
  // none) isn't offered at all, rather than saying what it needs.
  const inactive = new Set(parts.filter((p) => runs?.get(p.id)?.run?.inactive).map((p) => p.id));
  const lensParts = parts.filter((p) => p.lens && !inactive.has(p.id)).map((p) => ({ part: p, run: runs?.get(p.id)?.run ?? null, error: runs?.get(p.id)?.error ?? null }));
  const empty = new Set(runs ? foldEmpty(lensParts).empty.map((e) => e.part.id) : []);
  const folded = parts.filter((p) => empty.has(p.id));
  return (
    <div data-testid="lens-document">
      {parts
        .filter((p) => !empty.has(p.id) && !inactive.has(p.id))
        .map((part) => {
          const run = runs?.get(part.id);
          return (
            <Section
              key={part.id}
              id={part.id}
              title={<PartTitle part={part} run={run?.run ?? null} onOpenPage={onOpenPage} />}
              source={part.from ?? undefined}
              testId={`lens-part-${part.id}`}
            >
              <PartBody part={part} run={run} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
            </Section>
          );
        })}
      {folded.length > 0 ? (
        <p data-testid="lens-document-nothing-found" className="ox-empty-line">
          Nothing found: {folded.map((p) => p.title ?? runs?.get(p.id)?.run?.lens.title ?? p.id).join(", ")}.
        </p>
      ) : null}
    </div>
  );
}

/** `dashboard`: its parts as tiles at their sizes, each a titled card. */
export function DashboardViz(props: PartsVizProps) {
  const runs = usePartRuns(props);
  const { parts, streamId, onOpenPage, params, onSetParam } = props;
  return (
    <TileGrid testId="lens-dashboard">
      {parts.map((part) => {
        const run = runs?.get(part.id);
        return (
          <TileSlot key={part.id} size={part.size ?? (part.markdown !== null ? "full" : "small")} data-testid={`lens-part-${part.id}`}>
            <Tile title={<PartTitle part={part} run={run?.run ?? null} onOpenPage={onOpenPage} />}>
              <PartBody part={part} run={run} maxRows={TILE_MAX_ROWS} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
            </Tile>
          </TileSlot>
        );
      })}
    </TileGrid>
  );
}

const titleLinkStyle = {
  background: "none",
  border: "none",
  padding: 0,
  font: "inherit",
  color: "inherit",
  cursor: "pointer",
  textAlign: "left",
} as const;
