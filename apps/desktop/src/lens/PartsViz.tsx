import type { PickedRow } from "./LensRail.js";
import { useCallback, useEffect, useId, useMemo, useState, type CSSProperties, type ReactNode } from "react";

import { listMetricDefinitions, runLensAction, type LensRun, type MetricSpec, type SqlCell } from "../api.js";
import type { LensAction, LensPart, Reads } from "../tauri-bridge/generated/bindings.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { MetricTileBody } from "../components/Dashboard/MetricTile.js";
import { QueryTileBody } from "../components/Dashboard/QueryTile.js";
import { BAND_STYLE, TextBand } from "../components/Dashboard/TextTile.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { LENS_ROW_DRAG_MIME } from "../dragMimes.js";
import { resolveRange } from "../kit/rangePresets.js";
import type { MenuItem } from "../menu.js";
import { Replaceable } from "../components/Replaceable.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import { Loading } from "../kit/Loading.js";
import { Section } from "../kit/Section.js";
import { Tile, TileGrid, TileSlot } from "../kit/Tiles.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { LensResultView, type LensActionsState } from "./LensResultView.js";
import { actionOffered } from "./lensActions.js";
import { foldEmpty, rowAsk, rowTiles, type RowTile } from "./lensModel.js";
import { NO_READS, useRerunOnChange } from "./lensRerun.js";
import { dragActionFor, dropBeside, dropInGroup, keyAction, withOrder, type DropAt } from "./rowInteract.js";
import { useFormSubmit } from "./useFormSubmit.js";
import { useLensRuns, type PartRun } from "./useLensRuns.js";

const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };

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
  const own = <LensResultView run={run.run} onOpenPage={onOpenPage} streamId={streamId} maxRows={maxRows} toolbar={false} partButtons onSetParam={set} fitCustom={maxRows === undefined} />;
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
 * one closing line. A view document (`viewed`) is given its one section
 * in view and draws it as the page's body: no heading (the page's title
 * is the section's), its lens's description as the intro.
 */
export function DocumentViz(props: PartsVizProps & { viewed?: boolean; fills?: boolean }) {
  const runs = usePartRuns(props);
  const { parts, streamId, onOpenPage, params, onSetParam } = props;
  if (props.viewed) {
    return (
      <div data-testid="lens-document">
        {parts.map((part) => {
          const run = runs?.get(part.id);
          const intro = run?.run?.lens.description;
          return (
            <div key={part.id} data-testid={`lens-part-${part.id}`}>
              {intro ? <p className="ox-intro">{intro}</p> : null}
              <PartBody part={part} run={run} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
            </div>
          );
        })}
      </div>
    );
  }
  // A part that needs what isn't active (a link, where the work list keeps
  // none) isn't offered at all, rather than saying what it needs.
  const inactive = new Set(parts.filter((p) => runs?.get(p.id)?.run?.inactive).map((p) => p.id));
  // A part that declares a param the page's row names and has no value for
  // (an effort's lens on a page that shows no effort) has nothing to show
  // for this page: it is left out. (A page param that is unset is another
  // matter: the part shows what to pick.)
  const given = props.given ?? params;
  for (const p of parts) {
    const lens = runs?.get(p.id)?.run?.lens;
    if (lens?.params.some((q) => given[q.name] === null && params[q.name] !== null && q.name in given)) inactive.add(p.id);
  }
  const lensParts = parts
    .filter((p) => p.lens && !inactive.has(p.id))
    .map((p) => ({ part: p, run: runs?.get(p.id)?.run ?? null, error: runs?.get(p.id)?.error ?? null }));
  // A replaceable part stands for whatever replaces it, and a bare one has
  // no heading to hold: neither folds into "Nothing found".
  const stays = new Set(parts.filter((p) => p.replaceable || p.title === "").map((p) => p.id));
  const empty = new Set(runs ? foldEmpty(lensParts).empty.filter((e) => !stays.has(e.part.id)).map((e) => e.part.id) : []);
  const folded = parts.filter((p) => empty.has(p.id));
  // A document that fills (its last section's lens does: the loader says)
  // is a flex column whose last section takes the height left.
  const fills = props.fills ?? false;
  const last = parts[parts.length - 1]?.id;
  return (
    <div data-testid="lens-document" style={fills ? fillStyle : undefined}>
      {parts
        .filter((p) => !empty.has(p.id) && !inactive.has(p.id))
        .map((part) => {
          const run = runs?.get(part.id);
          const fill = fills && part.id === last;
          // A part titled "" is bare: its body, no heading.
          if (part.title === "") {
            return (
              <div key={part.id} data-testid={`lens-part-${part.id}`} style={fill ? fillStyle : undefined}>
                <PartBody part={part} run={run} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
              </div>
            );
          }
          return (
            <Section
              key={part.id}
              id={part.id}
              title={<PartTitle part={part} run={run?.run ?? null} onOpenPage={onOpenPage} />}
              source={part.from ?? undefined}
              testId={`lens-part-${part.id}`}
              fill={fill}
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

/** The metric definitions, when some tile shows the metric card: one
 *  fetch for the grid, re-read when they change. */
function useMetricDefs(wanted: boolean): Map<string, MetricSpec> {
  const [defs, setDefs] = useState<Map<string, MetricSpec>>(new Map());
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    if (!wanted) return;
    void listMetricDefinitions().then((specs) => {
      setDefs(new Map(specs.rows.map((d) => [d.key, d])));
      setReads(specs.reads);
    });
  }, [wanted]);
  useEffect(() => load(), [load]);
  useRerunOnChange(reads, load);
  return defs;
}

/** The column an edit action saves (`{{row.<column>}}` bound to its
 *  edited field), if any. */
function editColumn(action: LensAction): string | null {
  const bound = action.editField ? (action.input as Record<string, unknown> | null)?.[action.editField] : null;
  const m = typeof bound === "string" ? /^\{\{\s*row\.([^}|\s]+)\s*\}\}$/.exec(bound) : null;
  return m ? m[1]! : null;
}

interface DashboardVizProps extends PartsVizProps {
  /** The dashboard's own run: its rows are its tiles when it has `tile:`. */
  run: LensRun;
  /** Its actions, pressed from its row tiles. */
  actions?: LensActionsState;
}

/**
 * `dashboard`: its parts as tiles at their sizes, each a titled card, then
 * its row tiles (`tile:`): one per row, drawn by its lens, its SQL (as a
 * viz or the metric card) or its text (a heading band). A row tile is a
 * row: right-click offers its row actions, their keys run them, it drags
 * where a drag action runs (`{{order.*}}`: every tile in its new order),
 * and a heading is edited in place through the edit action. See
 * `.context/extensions.md` "Rows as tiles".
 */
export function DashboardViz(props: DashboardVizProps) {
  const { run, actions, parts, params, given, streamId, onOpenPage, onSetParam } = props;
  const lens = run.lens;
  const tiles = rowTiles(run);
  // Row tiles' lenses by tile id, sorted: a reorder runs nothing again.
  const rowLenses = tiles
    .filter((t) => t.kind === "lens")
    .map((t) => ({ key: `row:${t.id}`, lensId: t.lens! }))
    .sort((a, b) => a.key.localeCompare(b.key));
  const runs = useLensRuns([...parts.flatMap((p) => (p.lens ? [{ key: p.id, lensId: p.lens }] : [])), ...rowLenses], given ?? params, streamId);
  const defs = useMetricDefs(tiles.some((t) => t.kind === "metric"));
  // The metric cards' window: the page's `range` and `branch`, if it has them.
  const rangeKey = typeof params.range === "string" ? params.range : "all";
  const branch = typeof params.branch === "string" && params.branch !== "" ? params.branch : null;
  const window = useMemo(() => ({ range: resolveRange(rangeKey, Date.now()), branch }), [rangeKey, branch]);
  const ctxMenu = useContextMenu();
  const dragToken = useId();
  const [hint, setHint] = useState<{ id: string; zone: "before" | "after" } | null>(null);
  const dragActions = lens.actions.filter((a) => a.drag);
  const rowActions = lens.actions.filter((a) => a.row && !a.edit && !a.drag);
  const edit = lens.actions.find((a) => a.edit && lens.tile?.text && editColumn(a) === lens.tile.text) ?? null;
  const records = tiles.map((t) => t.record);

  const carriesTile = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes(LENS_ROW_DRAG_MIME);
  const draggedTile = (e: React.DragEvent): RowTile | null => {
    try {
      const p = JSON.parse(e.dataTransfer.getData(LENS_ROW_DRAG_MIME)) as { token?: unknown; keys?: unknown };
      if (p.token !== dragToken || !Array.isArray(p.keys)) return null;
      return tiles.find((t) => t.id === p.keys[0]) ?? null;
    } catch {
      return null;
    }
  };
  // A drop runs the drag action with every tile in its new order.
  const dropTile = (dragged: RowTile, place: DropAt) => {
    const at = withOrder(place, records, [dragged.record]);
    const action = dragActionFor(dragActions, dragged.record, at);
    if (action && actions) actions.runDrops(action, [{ row: dragged.record, drop: at }]);
  };
  const others = (dragged: RowTile) => tiles.filter((t) => t.id !== dragged.id);
  const sideOf = (e: React.DragEvent): "before" | "after" => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return e.clientX < r.left + r.width / 2 ? "before" : "after";
  };

  const itemsFor = (tile: RowTile): MenuItem[] => [
    { id: "ask-about-row", label: "Ask About This", enabled: true, run: () => insertIntoAgent(rowAsk(lens, run.result.columns, run.result.rows[tiles.indexOf(tile)] ?? [])) },
    ...rowActions
      .filter((a) => actionOffered(a, tile.record))
      .map((a) => ({ id: `lens-action-${a.id}`, label: a.label, enabled: actions !== undefined, run: () => actions?.run(a, tile.record) })),
  ];

  const slotProps = (tile: RowTile): Record<string, unknown> => {
    const drags = dragActions.some((a) => actionOffered(a, tile.record));
    return {
      tabIndex: 0,
      "data-drop": hint?.id === tile.id ? hint.zone : undefined,
      onContextMenu: (e: React.MouseEvent) => ctxMenu.open(e, itemsFor(tile)),
      onKeyDown: (e: React.KeyboardEvent) => {
        if (e.target !== e.currentTarget) return;
        const keyed = e.metaKey || e.ctrlKey || e.altKey ? null : keyAction(lens.actions, e.key);
        if (keyed && actions && actionOffered(keyed, tile.record)) {
          e.preventDefault();
          actions.run(keyed, tile.record);
          return;
        }
        ctxMenu.openForKey(e, itemsFor(tile));
      },
      ...(drags
        ? {
            draggable: true,
            onDragStart: (e: React.DragEvent) => {
              e.dataTransfer.setData(LENS_ROW_DRAG_MIME, JSON.stringify({ token: dragToken, keys: [tile.id] }));
              e.dataTransfer.effectAllowed = "move";
            },
          }
        : {}),
      ...(dragActions.length > 0
        ? {
            onDragOver: (e: React.DragEvent) => {
              if (!carriesTile(e)) return;
              e.preventDefault();
              e.stopPropagation();
              const zone = sideOf(e);
              if (hint?.id !== tile.id || hint.zone !== zone) setHint({ id: tile.id, zone });
            },
            onDragLeave: () => setHint((h) => (h?.id === tile.id ? null : h)),
            onDrop: (e: React.DragEvent) => {
              if (!carriesTile(e)) return;
              e.preventDefault();
              e.stopPropagation();
              setHint(null);
              const dragged = draggedTile(e);
              if (!dragged || dragged.id === tile.id) return;
              const list = others(dragged);
              dropTile(dragged, dropBeside(list.map((t) => t.record), list.indexOf(tile), sideOf(e), null));
            },
          }
        : {}),
    };
  };
  // A drop on the grid between or past the tiles puts the tile last.
  const gridDrop =
    dragActions.length > 0
      ? {
          onDragOver: (e: React.DragEvent) => {
            if (carriesTile(e)) e.preventDefault();
          },
          onDrop: (e: React.DragEvent) => {
            if (!carriesTile(e)) return;
            e.preventDefault();
            setHint(null);
            const dragged = draggedTile(e);
            if (dragged) dropTile(dragged, dropInGroup(others(dragged).map((t) => t.record), null));
          },
        }
      : {};

  // Nothing to draw (no static tile, no row): the lens's empty line, which
  // `empty: ""` leaves out.
  if (parts.length === 0 && tiles.length === 0) {
    return lens.empty === "" ? null : (
      <p data-testid="lens-empty" style={{ color: "var(--text-secondary)" }}>
        {lens.empty ?? "No tiles."}
      </p>
    );
  }
  return (
    <TileGrid testId="lens-dashboard" {...gridDrop}>
      {parts.map((part) => {
        const partRun = runs?.get(part.id);
        return (
          <TileSlot key={part.id} size={part.size ?? (part.markdown !== null ? "full" : "small")} data-testid={`lens-part-${part.id}`}>
            <Tile title={<PartTitle part={part} run={partRun?.run ?? null} onOpenPage={onOpenPage} />}>
              <PartBody part={part} run={partRun} maxRows={TILE_MAX_ROWS} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
            </Tile>
          </TileSlot>
        );
      })}
      {tiles.map((tile) => (
        <TileSlot key={`row:${tile.id}`} size={tile.size} data-testid={`lens-tile-${tile.id}`} {...slotProps(tile)}>
          <RowTileBody
            tile={tile}
            run={runs?.get(`row:${tile.id}`)}
            def={tile.metric ? (defs.get(tile.metric) ?? null) : null}
            window={window}
            edit={edit}
            dashboard={run}
            streamId={streamId}
            onOpenPage={onOpenPage}
            params={params}
            onSetParam={onSetParam}
          />
        </TileSlot>
      ))}
      {ctxMenu.menu}
    </TileGrid>
  );
}

/** A row tile's card (or heading band) by its kind. */
function RowTileBody({
  tile,
  run,
  def,
  window,
  edit,
  dashboard,
  streamId,
  onOpenPage,
  params,
  onSetParam,
}: {
  tile: RowTile;
  run: PartRun | undefined;
  def: MetricSpec | null;
  window: { range: ReturnType<typeof resolveRange>; branch: string | null };
  /** The lens's edit action, when it saves the tiles' text. */
  edit: LensAction | null;
  dashboard: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  params: Record<string, SqlCell>;
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
}) {
  const field = edit?.editField ?? "";
  const form = useFormSubmit(
    edit?.label ?? "Save",
    (input, confirmed) => runLensAction(dashboard.lens.id, edit!.id, dashboard.params, tile.record, streamId, confirmed, input),
    () => {},
  );
  switch (tile.kind) {
    case "lens": {
      const part: LensPart = { id: tile.id, title: tile.title, lens: tile.lens, markdown: null, size: tile.size, replaceable: null, from: null };
      return (
        <Tile title={<PartTitle part={part} run={run?.run ?? null} onOpenPage={onOpenPage} />}>
          <PartBody part={part} run={run} maxRows={TILE_MAX_ROWS} streamId={streamId} onOpenPage={onOpenPage} params={params} onSetParam={onSetParam} />
        </Tile>
      );
    }
    case "sql":
      return (
        <Tile title={tile.title ?? "Query"}>
          <QueryTileBody sql={tile.sql!} display={tile.display} chart={tile.chart} maxRows={TILE_MAX_ROWS} onOpenPage={onOpenPage} />
        </Tile>
      );
    case "metric":
      return (
        <Tile title={tile.title ?? def?.title ?? tile.metric ?? "Metric"}>
          <MetricTileBody sql={tile.sql} metricKey={tile.metric} viz={tile.viz} size={tile.size} def={def} window={window} />
        </Tile>
      );
    case "text":
      return (
        <div style={BAND_STYLE}>
          {form.pending && edit ? (
            <CommandConfirm label={edit.label} command={edit.command} onConfirm={() => void form.submit(form.pending!, true)} onCancel={form.cancel} testIdPrefix={`lens-tile-${tile.id}-confirm`} />
          ) : null}
          <TextBand
            text={tile.text ?? ""}
            onCommit={edit ? (next) => next !== (tile.text ?? "") && void form.submit({ [field]: next }, false) : undefined}
            testId={`lens-tile-${tile.id}-text`}
          />
        </div>
      );
  }
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
