import type { ReactNode } from "react";

import type { LensRun, SqlCell } from "../api.js";
import type { LensPart } from "../tauri-bridge/generated/bindings.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
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
/** Rows a document section shows. */
const SECTION_MAX_ROWS = 25;

interface PartsVizProps {
  parts: readonly LensPart[];
  params: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
}

/** The lens parts of `parts`, run. */
function usePartRuns({ parts, params, streamId }: PartsVizProps) {
  return useLensRuns(
    parts.flatMap((p) => (p.lens ? [{ key: p.id, lensId: p.lens }] : [])),
    params,
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
function PartBody({ part, run, maxRows, streamId, onOpenPage }: { part: LensPart; run: PartRun | undefined; maxRows: number; streamId: string | null; onOpenPage?(ref: TabRef): void }): ReactNode {
  if (part.markdown !== null) return <MarkdownView body={part.markdown} />;
  if (!run) return <Loading what={part.title ?? "this part"} />;
  if (run.error) return <p className="ox-error">{run.error}</p>;
  return run.run ? <LensResultView run={run.run} onOpenPage={onOpenPage} streamId={streamId} maxRows={maxRows} toolbar={false} /> : null;
}

/**
 * `document`: its parts as sections, wiki style — a heading over a rule,
 * no boxes. What other extensions mounted is among them (placed by the
 * catalog) and says whose it is. Sections with nothing to show fold into
 * one closing line (tsk1036).
 */
export function DocumentViz(props: PartsVizProps) {
  const runs = usePartRuns(props);
  const { parts, streamId, onOpenPage } = props;
  const lensParts = parts.filter((p) => p.lens).map((p) => ({ part: p, run: runs?.get(p.id)?.run ?? null, error: runs?.get(p.id)?.error ?? null }));
  const empty = new Set(runs ? foldEmpty(lensParts).empty.map((e) => e.part.id) : []);
  const folded = parts.filter((p) => empty.has(p.id));
  return (
    <div data-testid="lens-document">
      {parts
        .filter((p) => !empty.has(p.id))
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
              <PartBody part={part} run={run} maxRows={SECTION_MAX_ROWS} streamId={streamId} onOpenPage={onOpenPage} />
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
  const { parts, streamId, onOpenPage } = props;
  return (
    <TileGrid testId="lens-dashboard">
      {parts.map((part) => {
        const run = runs?.get(part.id);
        return (
          <TileSlot key={part.id} size={part.size ?? (part.markdown !== null ? "full" : "small")} data-testid={`lens-part-${part.id}`}>
            <Tile title={<PartTitle part={part} run={run?.run ?? null} onOpenPage={onOpenPage} />}>
              <PartBody part={part} run={run} maxRows={TILE_MAX_ROWS} streamId={streamId} onOpenPage={onOpenPage} />
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
