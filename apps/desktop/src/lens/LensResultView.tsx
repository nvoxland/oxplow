import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { getLens, runLens, runLensAction, type LensRun, type SqlCell } from "../api.js";
import { DailyBarChart } from "../components/Analytics/DailyBarChart.js";
import { TrendChart } from "../components/charts/TrendChart.js";
import { squarify } from "../components/charts/squarify.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  barRows,
  cellLinkRef,
  childParams,
  displayColumns,
  formatCell,
  limitRows,
  lineSeries,
  rowMention,
  treemapItems,
  type DisplayColumn,
} from "./lensModel.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { showToast } from "../components/toastStore.js";
import { performLensAction } from "./lensActions.js";

type CellRenderer = (row: SqlCell[], col: DisplayColumn) => ReactNode;

/**
 * Renders a lens run's result in the lens's viz. Shared by the lens page,
 * slots and dashboard lens tiles; `maxRows` caps rows for compact views.
 * A `grid` runs its child lenses with this run's params, in `streamId`.
 */
export function LensResultView(props: LensResultViewProps) {
  const { run, streamId = null, compact = false } = props;
  // Compact strips (a number inline) have no room for buttons.
  if (compact || run.lens.actions.length === 0) return <LensBody {...props} />;
  return (
    <div>
      <LensActions run={run} streamId={streamId} />
      <LensBody {...props} />
    </div>
  );
}

interface LensResultViewProps {
  run: LensRun;
  /** Where links go; without it they navigate through the route context. */
  onOpenPage?(ref: TabRef): void;
  maxRows?: number;
  streamId?: string | null;
  /** Small inline rendering for strips (a number as plain text). */
  compact?: boolean;
}

/** The lens's declared buttons, above its result. */
function LensActions({ run, streamId }: { run: LensRun; streamId: string | null }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  return (
    <div data-testid="lens-actions" style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginBottom: 6 }}>
      {run.lens.actions.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`lens-action-${a.id}`}
          disabled={busy !== null}
          title={a.kind === "run-source" ? `Syncs ${a.source ?? ""}` : undefined}
          onClick={() => {
            setBusy(a.id);
            void performLensAction(a, run, streamId, {
              runLensAction,
              copyText: (t) => navigator.clipboard.writeText(t),
              insertIntoAgent,
              toast: (message) => showToast({ message }),
              recordError: (label, message) => recordOpError({ label, message }),
            })
              .then((outcome) => {
                if (a.kind === "copy" && outcome === "done") {
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1500);
                }
              })
              .finally(() => setBusy(null));
          }}
        >
          {busy === a.id && a.kind === "run-source" ? "Syncing…" : a.kind === "copy" && copied ? "Copied" : a.label}
        </button>
      ))}
    </div>
  );
}

function LensBody({ run, onOpenPage, maxRows, streamId = null, compact = false }: LensResultViewProps) {
  const lens = run.lens;
  if (lens.viz === "grid") {
    return <GridViz childIds={lens.children} params={run.params} streamId={streamId} onOpenPage={onOpenPage} />;
  }
  const result = limitRows(run.result, maxRows);
  const ctxMenu = useContextMenu();
  if (result.rows.length === 0) {
    return (
      <p data-testid="lens-empty" style={{ color: "var(--text-secondary)" }}>
        {lens.empty ?? "No rows."}
      </p>
    );
  }
  const cols = displayColumns(lens, result.columns);
  const cell: CellRenderer = (row, c) => {
    const text = formatCell(row[c.index] ?? null);
    const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
    if (!ref) return text;
    return (
      <RouteLink to={ref} onNavigate={onOpenPage ? () => onOpenPage(ref) : undefined} style={linkStyle}>
        {text}
      </RouteLink>
    );
  };
  const first = result.rows[0]?.[0] ?? null;
  const onRowMenu = (e: React.MouseEvent, row: SqlCell[]) =>
    ctxMenu.open(e, [
      {
        id: "add-row-to-agent",
        label: "Add Row to Agent Context",
        enabled: true,
        run: () => insertIntoAgent(rowMention(lens.id, result.columns, row)),
      },
    ]);
  switch (lens.viz) {
    case "bar":
      return (
        <div data-testid="lens-bar">
          <DailyBarChart rows={barRows(lens, result)} formatValue={(v) => formatMetricValue(v)} />
        </div>
      );
    case "line":
      return <LineViz run={{ ...run, result }} />;
    case "treemap":
      return <TreemapViz run={{ ...run, result }} onOpenPage={onOpenPage} />;
    case "number":
      return <NumberViz value={first} compact={compact} />;
    case "markdown":
      return <MarkdownViz body={first === null ? "" : String(first)} />;
    case "list":
      return (
        <>
          <ListViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} onRowMenu={onRowMenu} />
          {ctxMenu.menu}
        </>
      );
    case "table":
    default:
      return (
        <>
          <TableViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} onRowMenu={onRowMenu} />
          {ctxMenu.menu}
        </>
      );
  }
}

function MarkdownViz({ body }: { body: string }) {
  return (
    <div data-testid="lens-markdown">
      <MarkdownView body={body} />
    </div>
  );
}

function LineViz({ run }: { run: LensRun }) {
  const series = lineSeries(run.lens, run.result);
  return (
    <div data-testid="lens-line">
      {series.map((s) => (
        <div key={s.name} data-testid="lens-line-series" style={{ marginBottom: 12 }}>
          {s.name ? <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{s.name}</div> : null}
          <TrendChart points={s.points} width={640} height={180} />
        </div>
      ))}
    </div>
  );
}

const TREEMAP_W = 800;
const TREEMAP_H = 280;
const TREEMAP_PALETTE = ["#4e79a7", "#f28e2b", "#59a14f", "#e15759", "#76b7b2", "#edc948", "#b07aa1", "#9c755f"];

/** Two-level squarified treemap: groups first (by total size), then
 *  items inside each group. A tile follows the lens's first column link. */
function TreemapViz({ run, onOpenPage }: { run: LensRun; onOpenPage?(ref: TabRef): void }) {
  const items = treemapItems(run.lens, run.result);
  const groups = new Map<string, typeof items>();
  for (const it of items) {
    const g = it.group ?? "";
    groups.set(g, [...(groups.get(g) ?? []), it]);
  }
  const groupNames = [...groups.keys()];
  const color = (g: string) => TREEMAP_PALETTE[groupNames.indexOf(g) % TREEMAP_PALETTE.length]!;
  const groupRects = squarify(
    groupNames.map((g) => ({ value: groups.get(g)!.reduce((n, i) => n + i.size, 0), payload: g })),
    0,
    0,
    TREEMAP_W,
    TREEMAP_H,
  );
  const linkCol = run.lens.columns.find((c) => c.link);
  const open = (row: SqlCell[]) => {
    if (!linkCol?.link) return;
    const ref = cellLinkRef(linkCol.link, linkCol.key, row, run.result.columns);
    if (ref) onOpenPage?.(ref);
  };
  return (
    <div data-testid="lens-treemap">
      <svg viewBox={`0 0 ${TREEMAP_W} ${TREEMAP_H}`} style={{ width: "100%", maxWidth: TREEMAP_W }}>
        {groupRects.flatMap((g) =>
          squarify(
            groups.get(g.payload)!.map((it) => ({ value: it.size, payload: it })),
            g.x,
            g.y,
            g.w,
            g.h,
          ).map((t, i) => (
            <g
              key={`${g.payload}-${i}`}
              data-testid="lens-treemap-tile"
              onClick={() => open(t.payload.row)}
              style={{ cursor: linkCol ? "pointer" : "default" }}
            >
              <title>{`${t.payload.label}: ${formatMetricValue(t.payload.size)}${t.payload.group ? ` (${t.payload.group})` : ""}`}</title>
              <rect
                x={t.x}
                y={t.y}
                width={t.w}
                height={t.h}
                fill={color(g.payload)}
                stroke="var(--surface-page, #111)"
                strokeWidth={1}
                opacity={0.85}
              />
              {t.w > 60 && t.h > 16 ? (
                <text x={t.x + 4} y={t.y + 13} fontSize={11} fill="#fff" style={{ pointerEvents: "none" }}>
                  {t.payload.label.length > t.w / 7 ? `${t.payload.label.slice(0, Math.floor(t.w / 7) - 1)}…` : t.payload.label}
                </text>
              ) : null}
            </g>
          )),
        )}
      </svg>
      {groupNames.length > 1 || groupNames[0] ? (
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap", fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>
          {groupNames.map((g) => (
            <span key={g}>
              <span style={{ display: "inline-block", width: 10, height: 10, background: color(g), marginRight: 4 }} />
              {g || "—"}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** `grid`: each child lens, run with the params it declares from ours. */
function GridViz({
  childIds,
  params,
  streamId,
  onOpenPage,
}: {
  childIds: string[];
  params: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
}) {
  const [children, setChildren] = useState<{ id: string; run: LensRun | null; error: string | null }[]>([]);
  const paramsKey = JSON.stringify(params);
  useEffect(() => {
    let live = true;
    void Promise.all(
      childIds.map(async (id) => {
        try {
          const child = await getLens(id, streamId);
          return { id, run: await runLens(id, childParams(child, params), streamId), error: null };
        } catch (e) {
          return { id, run: null, error: e instanceof Error ? e.message : String(e) };
        }
      }),
    ).then((next) => {
      if (live) setChildren(next);
    });
    return () => {
      live = false;
    };
    // paramsKey stands in for `params` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [childIds.join("|"), paramsKey, streamId]);
  return (
    <div data-testid="lens-grid">
      {children.map(({ id, run, error }) => (
        <section key={id} data-testid={`lens-grid-child-${id}`} style={{ marginBottom: 20 }}>
          <h3 style={{ fontSize: "var(--text-sm)", margin: "0 0 6px" }}>{run?.lens.title ?? id}</h3>
          {error ? (
            <div style={{ fontSize: "var(--text-xs)", color: "var(--severity-critical)" }}>{error}</div>
          ) : run ? (
            <LensResultView run={run} onOpenPage={onOpenPage} streamId={streamId} maxRows={25} />
          ) : null}
        </section>
      ))}
    </div>
  );
}

function NumberViz({ value, compact }: { value: SqlCell; compact: boolean }) {
  return (
    <div
      data-testid="lens-number"
      title={typeof value === "number" ? formatMetricValueExact(value) : undefined}
      style={
        compact
          ? { display: "inline", color: "var(--text-secondary)", fontWeight: 500 }
          : { fontSize: 48, fontWeight: 600, margin: "16px 0" }
      }
    >
      {typeof value === "number" ? formatMetricValue(value) : formatCell(value)}
    </div>
  );
}

interface RowsVizProps {
  rows: SqlCell[][];
  cols: DisplayColumn[];
  cell: CellRenderer;
  truncated: boolean;
  /** Right-click on a row (per-row actions are right-click only). */
  onRowMenu(e: React.MouseEvent, row: SqlCell[]): void;
}

function TruncatedNote({ rows, truncated }: { rows: number; truncated: boolean }) {
  if (!truncated) return null;
  return (
    <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Showing the first {rows} rows.</p>
  );
}

function ListViz({ rows, cols, cell, truncated, onRowMenu }: RowsVizProps) {
  const [head, ...rest] = cols;
  return (
    <>
      <ul data-testid="lens-list" style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {rows.map((row, i) => (
          <li key={i} data-testid={`lens-row-${i}`} style={listRowStyle} onContextMenu={(e) => onRowMenu(e, row)}>
            <div>{head ? cell(row, head) : null}</div>
            {rest.length > 0 ? (
              <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
                {rest.map((c, j) => (
                  <span key={c.key}>
                    {j > 0 ? " · " : null}
                    {cell(row, c)}
                  </span>
                ))}
              </div>
            ) : null}
          </li>
        ))}
      </ul>
      <TruncatedNote rows={rows.length} truncated={truncated} />
    </>
  );
}

function TableViz({ rows, cols, cell, truncated, onRowMenu }: RowsVizProps) {
  return (
    <>
      <table data-testid="lens-table" style={tableStyle}>
        <thead>
          <tr>
            {cols.map((c) => (
              <th key={c.key} style={thStyle}>
                {c.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} data-testid={`lens-row-${i}`} onContextMenu={(e) => onRowMenu(e, row)}>
              {cols.map((c) => (
                <td key={c.key} style={tdStyle}>
                  {cell(row, c)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <TruncatedNote rows={rows.length} truncated={truncated} />
    </>
  );
}

const tableStyle: CSSProperties = { width: "100%", borderCollapse: "collapse", fontSize: "var(--text-sm)" };
const thStyle: CSSProperties = {
  textAlign: "left",
  padding: "6px 8px",
  borderBottom: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
  fontWeight: 600,
};
const tdStyle: CSSProperties = { padding: "6px 8px", borderBottom: "1px solid var(--border-subtle)", verticalAlign: "top" };
const listRowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)" };
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-link, var(--accent))",
  cursor: "pointer",
  textAlign: "left",
  font: "inherit",
};
