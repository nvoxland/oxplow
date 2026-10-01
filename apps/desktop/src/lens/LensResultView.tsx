import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { getLens, lensForm, lensText, runLens, runLensAction, submitLensForm, type LensRun, type SqlCell } from "../api.js";
import type { FormStart } from "../tauri-bridge/generated/bindings.js";
import type { LensAction } from "../tauri-bridge/generated/bindings.js";
import { DailyBarChart } from "../components/Analytics/DailyBarChart.js";
import { TrendChart } from "../components/charts/TrendChart.js";
import { squarify } from "../components/charts/squarify.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { DiffPane } from "../components/Diff/DiffPane.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { SchemaForm } from "../components/SchemaForm/SchemaForm.js";
import { needsConfirmation } from "../ipc-error.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  barRows,
  cellLinkRef,
  childParams,
  displayColumns,
  formatCell,
  hunkRows,
  limitRows,
  lineSeries,
  rowAsk,
  rowRef,
  stepItems,
  timelineEntries,
  treeNodes,
  treemapItems,
  type DisplayColumn,
  type StepStatus,
  type TreeNode,
} from "./lensModel.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { uiCommandMenuItems, uiCommandsAbout } from "../components/uiCommands.js";
import { useUiCommands } from "../components/useUiCommands.js";
import { personCommands } from "../personCommands.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { showToast } from "../components/toastStore.js";
import { addLensToContext, copyLens, performLensAction, rowRecord } from "./lensActions.js";

type CellRenderer = (row: SqlCell[], col: DisplayColumn) => ReactNode;

/**
 * Renders a lens run's result in the lens's viz. Shared by the lens page,
 * slots and dashboard lens tiles; `maxRows` caps rows for compact views.
 * A `grid` runs its child lenses with this run's params, in `streamId`.
 * Every lens has Copy and Add to Agent Context; its declared actions run
 * their commands as the lens (row actions from a row's right-click menu).
 */
export function LensResultView(props: LensResultViewProps) {
  const { run, streamId = null, compact = false, toolbar = true } = props;
  const actions = useLensActions(run, streamId);
  // A command's confirmation shows wherever the row is — an answer in the
  // strip, a grid child, a dashboard tile — not only where the toolbar is.
  const confirm = actions.pending ? (
    <CommandConfirm
      label={actions.pending.action.label}
      command={actions.pending.action.command}
      onConfirm={actions.confirm}
      onCancel={actions.cancel}
      testIdPrefix="lens-action-confirm"
    />
  ) : null;
  // Compact strips (a number inline) have no room for buttons.
  if (compact || !toolbar) {
    return (
      <>
        {confirm}
        <LensBody {...props} runRowAction={actions.run} />
      </>
    );
  }
  return (
    <div>
      {confirm}
      <LensToolbar run={run} streamId={streamId} actions={actions} />
      <LensBody {...props} runRowAction={actions.run} />
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
  /** Show the toolbar (Copy, Add to Agent Context, actions); a grid's
   *  children leave it to the grid. Default true. */
  toolbar?: boolean;
}

/** Pressing a lens's actions, with the confirmation a command may ask
 *  for held until the person answers. */
function useLensActions(run: LensRun, streamId: string | null) {
  const [busy, setBusy] = useState<string | null>(null);
  const [pending, setPending] = useState<{ action: LensAction; row: Record<string, SqlCell> | null } | null>(null);
  const deps = {
    runLensAction,
    toast: (message: string) => showToast({ message }),
    recordError: (label: string, message: string) => recordOpError({ label, message }),
  };
  const press = async (action: LensAction, row: Record<string, SqlCell> | null, confirmed: boolean) => {
    setBusy(action.id);
    try {
      const outcome = await performLensAction(action, run, row, streamId, confirmed, deps);
      setPending(outcome === "needs-confirmation" ? { action, row } : null);
    } finally {
      setBusy(null);
    }
  };
  return {
    busy,
    pending,
    run: (action: LensAction, row: Record<string, SqlCell> | null) => void press(action, row, false),
    confirm: () => {
      if (pending) void press(pending.action, pending.row, true);
    },
    cancel: () => setPending(null),
  };
}

type LensActionsState = ReturnType<typeof useLensActions>;

/** Copy, Add to Agent Context, and the lens's whole-lens actions. */
function LensToolbar({ run, streamId, actions }: { run: LensRun; streamId: string | null; actions: LensActionsState }) {
  const [copied, setCopied] = useState(false);
  const buttons = run.lens.actions.filter((a) => !a.row);
  return (
    <>
    <div data-testid="lens-actions" style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 6, marginBottom: 6 }}>
      <button
        type="button"
        data-testid="lens-copy"
        onClick={() => {
          void copyLens(run, streamId, {
            lensText,
            copyText: (t) => navigator.clipboard.writeText(t),
            recordError: (label, message) => recordOpError({ label, message }),
          }).then((ok) => {
            if (ok) {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            }
          });
        }}
      >
        {copied ? "Copied" : "Copy"}
      </button>
      <button type="button" data-testid="lens-add-to-context" onClick={() => addLensToContext(run, insertIntoAgent)}>
        Add to Agent Context
      </button>
      {buttons.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`lens-action-${a.id}`}
          disabled={actions.busy !== null}
          title={a.command}
          onClick={() => actions.run(a, null)}
        >
          {actions.busy === a.id ? `${a.label}…` : a.label}
        </button>
      ))}
    </div>
    </>
  );
}

type RowActionRunner = (action: LensAction, row: Record<string, SqlCell>) => void;

function LensBody(props: LensResultViewProps & { runRowAction?: RowActionRunner }) {
  const { run, onOpenPage, streamId = null } = props;
  // A grid composes child lenses and has no rows of its own; it's its own
  // component so the row views' hooks always run in the same order.
  if (run.lens.viz === "grid") {
    return <GridViz childIds={run.lens.children} params={run.params} streamId={streamId} onOpenPage={onOpenPage} />;
  }
  if (run.lens.viz === "form") return <FormViz run={run} streamId={streamId} />;
  return <RowsBody {...props} />;
}

function RowsBody({
  run,
  onOpenPage,
  maxRows,
  streamId = null,
  compact = false,
  runRowAction,
}: LensResultViewProps & { runRowAction?: RowActionRunner }) {
  const lens = run.lens;
  const result = limitRows(run.result, maxRows);
  const ctxMenu = useContextMenu();
  const uiCommands = useUiCommands(streamId);
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
  const rowActions = lens.actions.filter((a) => a.row);
  const rowItems = (row: SqlCell[]) => [
    {
      id: "ask-about-row",
      label: "Ask About This",
      enabled: true,
      run: () => insertIntoAgent(rowAsk(lens, result.columns, row)),
    },
    ...rowActions.map((a) => ({
      id: `lens-action-${a.id}`,
      label: a.label,
      enabled: runRowAction !== undefined,
      run: () => runRowAction?.(a, rowRecord(result.columns, row)),
    })),
    // Extensions' commands for what the row links to (P6b.C4).
    ...(() => {
      const ref = rowRef(lens, result.columns, row);
      return ref
        ? uiCommandMenuItems(uiCommandsAbout(uiCommands, ref, "context"), ref, (c, input) =>
            void personCommands.run(c.label, c.command, input),
          )
        : [];
    })(),
  ];
  // Every row of every row component: focusable, with its menu from a
  // right-click or the keyboard (Menu key / Shift+F10).
  const rowMenu: RowMenu = (row) => ({
    tabIndex: 0,
    onContextMenu: (e) => ctxMenu.open(e, rowItems(row)),
    onKeyDown: (e) => ctxMenu.openForKey(e, rowItems(row)),
  });
  return (
    <>
      <LensViz {...{ run, result, lens, cols, cell, first, compact, streamId, onOpenPage, rowMenu }} />
      {ctxMenu.menu}
    </>
  );
}

type RowMenu = (row: SqlCell[]) => {
  tabIndex: number;
  onContextMenu(e: React.MouseEvent): void;
  onKeyDown(e: React.KeyboardEvent): void;
};

function LensViz({
  run,
  result,
  lens,
  cols,
  cell,
  first,
  compact,
  streamId,
  onOpenPage,
  rowMenu,
}: {
  run: LensRun;
  result: LensRun["result"];
  lens: LensRun["lens"];
  cols: DisplayColumn[];
  cell: CellRenderer;
  first: SqlCell | null;
  compact: boolean;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  rowMenu: RowMenu;
}) {
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
    case "tree":
      return <TreeViz nodes={treeNodes(lens, result)} rowMenu={rowMenu} />;
    case "timeline":
      return <TimelineViz entries={timelineEntries(lens, result)} onOpenPage={onOpenPage} rowMenu={rowMenu} />;
    case "detail":
      return <DetailViz row={result.rows[0]!} cols={cols} cell={cell} rowMenu={rowMenu} />;
    case "steps":
      return <StepsViz steps={stepItems(lens, result)} rowMenu={rowMenu} />;
    case "hunks":
      return <HunksViz rows={hunkRows(lens, result)} streamId={streamId} rowMenu={rowMenu} />;
    case "list":
      return <ListViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} rowMenu={rowMenu} />;
    case "table":
    default:
      return <TableViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} rowMenu={rowMenu} />;
  }
}

/** `form`: the fields of its command's input, starting from the form's
 *  values; submitting runs the command as the lens, for the person. */
function FormViz({ run, streamId }: { run: LensRun; streamId: string | null }) {
  const [start, setStart] = useState<FormStart | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Record<string, unknown> | null>(null);
  const [round, setRound] = useState(0);
  const paramsKey = JSON.stringify(run.params);
  useEffect(() => {
    let live = true;
    lensForm(run.lens.id, run.params, streamId)
      .then((s) => {
        if (live) setStart(s);
      })
      .catch((e) => {
        if (live) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      live = false;
    };
    // paramsKey stands in for `run.params` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run.lens.id, paramsKey, streamId]);
  if (error) return <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)" }}>{error}</div>;
  if (!start) return null;
  const submit = async (input: Record<string, unknown>, confirmed: boolean) => {
    setBusy(true);
    try {
      await submitLensForm(run.lens.id, input, run.params, streamId, confirmed);
      setPending(null);
      showToast({ message: `${run.lens.title}: done.` });
      // A fresh form for the next entry.
      setRound((r) => r + 1);
    } catch (e) {
      if (!confirmed && needsConfirmation(e)) setPending(input);
      else recordOpError({ label: run.lens.title, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div data-testid="lens-form">
      {pending ? (
        <CommandConfirm
          label={run.lens.title}
          command={start.command.name}
          onConfirm={() => void submit(pending, true)}
          onCancel={() => setPending(null)}
          testIdPrefix="lens-form-confirm"
        />
      ) : null}
      <SchemaForm
        key={round}
        schema={start.command.input_schema as Record<string, unknown>}
        initial={start.values}
        submitLabel={run.lens.title}
        busy={busy}
        onSubmit={(input) => void submit(input, false)}
        testIdPrefix="lens-form-fields"
      />
    </div>
  );
}

/** `tree`: nested rows, each branch collapsible (expanded by default). */
function TreeViz({ nodes, rowMenu }: { nodes: TreeNode[]; rowMenu: RowMenu }) {
  return (
    <ul data-testid="lens-tree" style={treeListStyle}>
      {nodes.map((n, i) => (
        <TreeItem key={i} node={n} rowMenu={rowMenu} />
      ))}
    </ul>
  );
}

function TreeItem({ node, rowMenu }: { node: TreeNode; rowMenu: RowMenu }) {
  const [open, setOpen] = useState(true);
  const branch = node.children.length > 0;
  return (
    <li data-testid="lens-tree-node">
      <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "2px 0" }} {...rowMenu(node.row)}>
        {branch ? (
          <button
            type="button"
            aria-expanded={open}
            aria-label={open ? "Collapse" : "Expand"}
            onClick={() => setOpen((o) => !o)}
            style={disclosureStyle}
          >
            {open ? "▾" : "▸"}
          </button>
        ) : (
          <span style={{ display: "inline-block", width: 16 }} />
        )}
        <span>{node.label}</span>
      </div>
      {branch && open ? (
        <ul style={{ ...treeListStyle, paddingLeft: 16 }}>
          {node.children.map((c, i) => (
            <TreeItem key={i} node={c} rowMenu={rowMenu} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/** `timeline`: entries oldest first, each linked through its ref. */
function TimelineViz({
  entries,
  onOpenPage,
  rowMenu,
}: {
  entries: ReturnType<typeof timelineEntries>;
  onOpenPage?(ref: TabRef): void;
  rowMenu: RowMenu;
}) {
  return (
    <ol data-testid="lens-timeline" style={{ listStyle: "none", padding: 0, margin: 0, borderLeft: "2px solid var(--border-subtle)" }}>
      {entries.map((e, i) => {
        const ref = e.ref ? refFromTabId(e.ref) : null;
        return (
          <li key={i} data-testid={`lens-timeline-entry-${i}`} style={{ padding: "4px 0 4px 12px" }} {...rowMenu(e.row)}>
            <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{e.at}</div>
            <div>
              {ref ? (
                <RouteLink to={ref} onNavigate={onOpenPage ? () => onOpenPage(ref) : undefined} style={linkStyle}>
                  {e.label}
                </RouteLink>
              ) : (
                e.label
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** `detail`: the first row as label/value pairs. */
function DetailViz({ row, cols, cell, rowMenu }: { row: SqlCell[]; cols: DisplayColumn[]; cell: CellRenderer; rowMenu: RowMenu }) {
  return (
    <dl data-testid="lens-detail" style={{ display: "grid", gridTemplateColumns: "max-content 1fr", gap: "4px 12px", margin: 0 }} {...rowMenu(row)}>
      {cols.map((c) => (
        <div key={c.key} style={{ display: "contents" }}>
          <dt style={{ color: "var(--text-secondary)", fontWeight: 600 }}>{c.label}</dt>
          <dd style={{ margin: 0 }}>{cell(row, c)}</dd>
        </div>
      ))}
    </dl>
  );
}

const STEP_MARK: Record<StepStatus, { mark: string; color: string; label: string }> = {
  done: { mark: "✓", color: "var(--diff-add-fg)", label: "Done" },
  active: { mark: "▶", color: "var(--accent)", label: "In progress" },
  failed: { mark: "✗", color: "var(--severity-critical)", label: "Failed" },
  pending: { mark: "○", color: "var(--text-muted)", label: "Pending" },
};

/** `steps`: an ordered checklist. */
function StepsViz({ steps, rowMenu }: { steps: ReturnType<typeof stepItems>; rowMenu: RowMenu }) {
  return (
    <ol data-testid="lens-steps" style={{ listStyle: "none", padding: 0, margin: 0 }}>
      {steps.map((s, i) => {
        const m = STEP_MARK[s.status];
        return (
          <li key={i} data-testid={`lens-step-${i}`} data-status={s.status} style={{ display: "flex", gap: 8, padding: "3px 0" }} {...rowMenu(s.row)}>
            <span title={m.label} aria-label={m.label} style={{ color: m.color, width: 16, textAlign: "center" }}>
              {m.mark}
            </span>
            <span style={{ color: s.status === "pending" ? "var(--text-secondary)" : undefined }}>
              {i + 1}. {s.label}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/** `hunks`: each file and its revisions; expanding one shows its diff in
 *  the diff viewer (one at a time — each is an editor). */
function HunksViz({ rows, streamId, rowMenu }: { rows: ReturnType<typeof hunkRows>; streamId: string | null; rowMenu: RowMenu }) {
  const [open, setOpen] = useState<number | null>(rows.length === 1 ? 0 : null);
  return (
    <div data-testid="lens-hunks">
      {rows.map((r, i) => (
        <section key={i} data-testid={`lens-hunk-${i}`} style={{ borderBottom: "1px solid var(--border-subtle)" }} {...rowMenu(r.row)}>
          <button
            type="button"
            aria-expanded={open === i}
            onClick={() => setOpen((o) => (o === i ? null : i))}
            style={{ ...disclosureStyle, width: "100%", textAlign: "left", padding: "6px 0", display: "flex", gap: 6 }}
          >
            <span>{open === i ? "▾" : "▸"}</span>
            <span style={{ fontFamily: "var(--font-mono)" }}>{r.path}</span>
            <span style={{ color: "var(--text-secondary)" }}>
              {r.from} → {r.to}
            </span>
          </button>
          {open === i ? (
            <div style={{ height: 360 }}>
              <DiffPane
                streamId={streamId ?? ""}
                spec={{ path: r.path, leftVersion: r.from, rightVersion: r.to, baseLabel: r.from }}
                visible
              />
            </div>
          ) : null}
        </section>
      ))}
    </div>
  );
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
/** Group colours: the `--chart-*` series tokens (index.html). */
const TREEMAP_PALETTE = Array.from({ length: 8 }, (_, i) => `var(--chart-${i + 1})`);

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
                stroke="var(--surface-app)"
                strokeWidth={1}
                opacity={0.85}
              />
              {t.w > 60 && t.h > 16 ? (
                <text x={t.x + 4} y={t.y + 13} fontSize={11} fill="var(--chart-label)" style={{ pointerEvents: "none" }}>
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
            <LensResultView run={run} onOpenPage={onOpenPage} streamId={streamId} maxRows={25} toolbar={false} />
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
  rowMenu: RowMenu;
}

function TruncatedNote({ rows, truncated }: { rows: number; truncated: boolean }) {
  if (!truncated) return null;
  return (
    <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Showing the first {rows} rows.</p>
  );
}

function ListViz({ rows, cols, cell, truncated, rowMenu }: RowsVizProps) {
  const [head, ...rest] = cols;
  return (
    <>
      <ul data-testid="lens-list" style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {rows.map((row, i) => (
          <li key={i} data-testid={`lens-row-${i}`} style={listRowStyle} {...rowMenu(row)}>
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

function TableViz({ rows, cols, cell, truncated, rowMenu }: RowsVizProps) {
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
            <tr key={i} data-testid={`lens-row-${i}`} {...rowMenu(row)}>
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

const treeListStyle: CSSProperties = { listStyle: "none", padding: 0, margin: 0 };
const disclosureStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  width: 16,
  color: "var(--text-secondary)",
  cursor: "pointer",
  font: "inherit",
};
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
