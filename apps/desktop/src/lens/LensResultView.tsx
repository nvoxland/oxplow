import { useCollapsedSet, useDraft, type CollapsedSet } from "../tabs/useViewParam.js";
import { Approvals } from "../kit/Approvals.js";
import type { PageAction } from "../kit/actions.js";
import type { PickedRow } from "./LensRail.js";
import { Fragment, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { commandOpenAfter, lensActionForm, lensForm, lensText, runLensAction, submitLensForm, undoCommand, type LensRun, type SqlCell } from "../api.js";
import { Slideover } from "../components/Slideover.js";
import { InlinePromptStrip } from "../components/InlinePromptStrip.js";
import type { FormStart } from "../tauri-bridge/generated/bindings.js";
import type { Drop, LensAction } from "../tauri-bridge/generated/bindings.js";
import { DailyBarChart } from "../components/Analytics/DailyBarChart.js";
import { TrendChart } from "../components/charts/TrendChart.js";
import { Sparkline } from "../components/Sparkline.js";
import { squarify } from "../components/charts/squarify.js";
import { formatFullDateTime, formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { MarkdownViz } from "./MarkdownViz.js";
import { DiffPane } from "../components/Diff/DiffPane.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { ResultCompleteness } from "./ResultCompleteness.js";
import { useFormSubmit } from "./useFormSubmit.js";
import { SchemaForm } from "../components/SchemaForm/SchemaForm.js";
import { withSubfields, type Drafts, type Field } from "../components/SchemaForm/schemaFormModel.js";
import { NO_READS, useRerunOnChange } from "./lensRerun.js";
import { RouteLink, routeDispatch } from "../tabs/RouteLink.js";
import { linkOrigins } from "./linkOrigin.js";
import { useListNav } from "../kit/useListNav.js";
import { parseUri } from "../uri/uri.js";
import { openRefInWindow } from "../tabs/openRef.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  barRows,
  cellLinkRef,
  linkTargetId,
  type LinkTarget,
  displayColumns,
  formatCell,
  hunkRows,
  isTimestamp,
  markParts,
  limitRows,
  lineSeries,
  rowAsk,
  rowContextRef,
  rowDepth,
  foldedAway,
  rowEmphasized,
  rowGroups,
  rowRef,
  sparklineValues,
  stepItems,
  timelineEntries,
  treeNodes,
  lazyTreeNodes,
  lazyTreeProblem,
  drawnTreeRows,
  treemapItems,
  partParams,
  codeRows,
  diffRow,
  type DisplayColumn,
  type RowGroup,
  type StepStatus,
  type TreeNode,
} from "./lensModel.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { refCommandMenuItems, useRefOffers } from "../components/refCommands.js";
import { safeColor } from "../components/decorators.js";
import { useDecorations } from "../components/useDecorations.js";
import { RefBadge } from "../components/RefBadge.js";
import { CustomComponentViz } from "./CustomComponentViz.js";
import type { HostedView } from "./hostedView.js";
import { DashboardViz, DocumentViz } from "./PartsViz.js";
import { CodeViz } from "./CodeViz.js";
import { DiffViz } from "./DiffViz.js";
import { useLensRuns } from "./useLensRuns.js";
import { useLazyBranches } from "./useLazyBranches.js";
import { DataTable, type DataColumn, type RowProps } from "../kit/DataTable.js";
import { Facts } from "../kit/Facts.js";
import { Stat } from "../kit/Stat.js";
import { recordOpError } from "../opErrors.js";
import { showToast } from "../components/toastStore.js";
import { actionOffered, addLensToContext, copyLens, offeredWholeLens, openAfterAction, performLensAction, promptField, rowRecord } from "./lensActions.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import {
  chainedDrops,
  dragActionFor,
  dropBeside,
  dropInGroup,
  dropZone,
  keyAction,
  marksAfterClick,
  rowKey,
  shownMarks,
  takesOnto,
  withOrder,
  type DropAt,
} from "./rowInteract.js";
import { LENS_ROW_DRAG_MIME } from "../dragMimes.js";
import { LensIcon, lensIcon, toneColor } from "./lensIcons.js";
import { useExtensions } from "../extensionsStore.js";
import { setContextRefDrag } from "../agent-context-dnd.js";

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
  const marks = useMarks(run);
  // A lazy tree (`tree.lazy`): the folders open by their id, this step's
  // history state; each runs the lens again for its rows, which are then
  // rows of the lens like the root's — marked, dragged, counted.
  const lazyOpen = useCollapsedSet(`lens-tree.open:${run.lens.id}`);
  const lazy = run.lens.viz === "tree" && !!run.lens.tree?.lazy;
  const branches = useLazyBranches(run, lazy ? lazyOpen.ids : [], streamId, props.maxRows);
  const tree: LazyTree | null = lazy ? { nodes: lazyTreeNodes(run.lens, run.result, new Set(lazyOpen.ids), branches), open: lazyOpen } : null;
  const shown = tree ? { ...run, result: { ...run.result, rows: drawnTreeRows(tree.nodes) } } : run;
  // A command's confirmation shows wherever the row is — an answer in the
  // strip, a grid child, a dashboard tile — not only where the toolbar is.
  const confirm = actions.pending ? (
    <CommandConfirm
      label={actions.pending.action.label}
      command={actions.pending.action.command}
      details={actions.pending.details}
      onConfirm={actions.confirm}
      onCancel={actions.cancel}
      testIdPrefix="lens-action-confirm"
    />
  ) : null;
  // A form action's form: a slideover beside whatever pressed it, or a
  // prompt strip at the top of the lens when the form is one text field.
  const form = actions.form ? (
    <ActionForm run={run} streamId={streamId} pressed={actions.form} onClose={actions.closeForm} />
  ) : null;
  // Compact strips (a number inline) have no room for buttons; a page
  // that takes them draws them in its nav bar.
  if (compact || !toolbar || props.toolbarTo) {
    return (
      <>
        {confirm}
        {form}
        {props.toolbarTo && !compact ? <HandToolbar run={shown} streamId={streamId} actions={actions} marks={marks} to={props.toolbarTo} /> : null}
        {!compact && !props.toolbarTo ? <MarksBar run={shown} actions={actions} marks={marks} /> : null}
        {!compact && !props.toolbarTo && props.partButtons ? <PartButtons run={shown} actions={actions} /> : null}
        <LensBody {...props} run={shown} tree={tree} actions={actions} marks={marks} />
      </>
    );
  }
  return (
    <div>
      {confirm}
      {form}
      <LensToolbar run={shown} streamId={streamId} actions={actions} marks={marks} />
      <LensBody {...props} run={shown} tree={tree} actions={actions} marks={marks} />
    </div>
  );
}

/** A lazy tree's nodes (`lazyTreeNodes`, its open branches' rows spliced
 *  in) and the open set they toggle. */
interface LazyTree {
  nodes: TreeNode[];
  open: CollapsedSet;
}

/** The rows a person has marked (`rowInteract.ts`): their keys in the
 *  entry's history state, per lens, so Back keeps them; and where a
 *  Shift+click range starts. */
interface Marks {
  keys: string[];
  anchor: number | null;
  set(keys: string[], anchor: number | null): void;
}

const NO_MARKS: { keys: string[]; anchor: number | null } = { keys: [], anchor: null };

function useMarks(run: LensRun): Marks {
  const [state, setState] = useHistoryState(`marked:${run.lens.id}`, NO_MARKS);
  return { keys: state.keys, anchor: state.anchor, set: (keys, anchor) => setState({ keys, anchor }) };
}

/** The marked rows of `run`, in its order, as records. */
function markedRecords(run: LensRun, marks: Marks): Record<string, SqlCell>[] {
  const set = new Set(marks.keys);
  return run.result.rows.filter((r) => set.has(rowKey(r))).map((r) => rowRecord(run.result.columns, r));
}

/** The bulk actions (`bulk: true`) the marked rows are offered: an action
 *  any of them is offered (`when`), run on those. */
function bulkOffers(run: LensRun, marked: Record<string, SqlCell>[]): { action: LensAction; rows: Record<string, SqlCell>[] }[] {
  return run.lens.actions
    .filter((a) => a.bulk)
    .map((action) => ({ action, rows: marked.filter((r) => actionOffered(action, r)) }))
    .filter((o) => o.rows.length > 0);
}

/** While rows are marked: how many, the bulk actions, and Clear — in place
 *  of the lens's own buttons. */
function MarksBar({ run, actions, marks }: { run: LensRun; actions: LensActionsState; marks: Marks }) {
  const marked = markedRecords(run, marks);
  if (marked.length === 0) return null;
  return (
    <div data-testid="lens-marks" style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 6, marginBottom: 6 }}>
      <span data-testid="lens-marks-count" style={{ color: "var(--text-secondary)" }}>
        {marked.length} selected
      </span>
      {bulkOffers(run, marked).map(({ action, rows }) => (
        <button key={action.id} type="button" data-testid={`lens-bulk-${action.id}`} disabled={actions.busy !== null} title={action.command} onClick={() => actions.runMany(action, rows)}>
          {actions.busy === action.id ? `${action.label}…` : action.label}
        </button>
      ))}
      <button type="button" data-testid="lens-marks-clear" onClick={() => marks.set([], null)}>
        Clear
      </button>
    </div>
  );
}

interface LensResultViewProps {
  run: LensRun;
  /** Where links go; without it they navigate through the route context. */
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  maxRows?: number;
  streamId?: string | null;
  /** Small inline rendering for strips (a number as plain text). */
  compact?: boolean;
  /** Show the toolbar (Copy, Add to Agent Context, actions); a grid's
   *  children leave it to the grid. Default true. */
  toolbar?: boolean;
  /** For a `viz: custom` lens whose component can't be shown: told why,
   *  instead of the lens showing its table (a replacement falls back to
   *  the core component, outside its own frame). */
  onCustomFailure?(reason: string): void;
  /** For a `viz: custom` lens: its component said `ready`. */
  onCustomReady?(): void;
  /** For a `viz: custom` lens: its host's view, where the host shares
   *  one with its component (a replacement, a lens's own page). */
  componentView?: HostedView | null;
  /** Set one of the lens's params (a row click on a column that
   *  `select`s it, with the row it picked); without it, rows are no
   *  picks. */
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
  /** Hand Copy, Add to Agent Context and the whole-lens actions to the
   *  page (its nav bar) instead of drawing them above the body. */
  toolbarTo?(actions: PageAction[]): void;
  /** A document's part or a tile has no toolbar, but its whole-lens
   *  actions (an Add…) show as buttons above its body. */
  partButtons?: boolean;
}

/** Where a command's `ui.open_after` page opens from a lens: in the page
 *  the lens is on (in its place with `replace`), else the window. */
function useOpenAfter() {
  const nav = useOptionalPageNavigation();
  return {
    openAfter: commandOpenAfter,
    openPage: (page: string, replace: boolean) => void (nav?.openRef ?? openRefInWindow)(page, { replace }),
    closePage: nav?.close,
    recordError: (label: string, message: string) => recordOpError({ label, message }),
  };
}

/** Pressing a lens's actions, with the confirmation a command may ask
 *  for held until the person answers. */
/** One run of an action: the row it runs on, and where a dragged row
 *  landed. A bulk action or a drop of several rows is several. */
interface Press {
  row: Record<string, SqlCell> | null;
  drop: Drop | null;
}

function useLensActions(run: LensRun, streamId: string | null) {
  const openAfter = useOpenAfter();
  const [busy, setBusy] = useState<string | null>(null);
  // A command that asks: what it asks about, and the runs that wait on the
  // answer (one confirmation for a bulk action's runs).
  const [pending, setPending] = useState<{ action: LensAction; presses: Press[]; details: string[] } | null>(null);
  // A form action pressed: its form is open until it runs or is closed.
  const [form, setForm] = useState<{ action: LensAction; row: Record<string, SqlCell> | null } | null>(null);
  const deps = {
    runLensAction: (
      id: string,
      action: string,
      params: Record<string, SqlCell>,
      row: Record<string, SqlCell> | null,
      stream: string | null,
      confirmed: boolean,
      drop?: Drop | null,
    ) => runLensAction(id, action, params, row, stream, confirmed, null, drop ?? null),
    toast: (message: string, undo?: () => void) => showToast({ message, onUndo: undo }),
    // The person's click on Undo is their confirmation of it.
    undo: (auditId: number) => undoCommand(auditId, true),
    ...openAfter,
  };
  // The runs in order; a run that asks stops them, the rest waiting on
  // the person's answer. Only the last says it's done (one toast).
  const press = async (action: LensAction, presses: Press[], confirmed: boolean) => {
    setBusy(action.id);
    try {
      for (let i = 0; i < presses.length; i++) {
        const p = presses[i]!;
        const quiet = i < presses.length - 1 ? { ...deps, toast: () => {} } : deps;
        const outcome = await performLensAction(action, run, p.row, streamId, confirmed, quiet, p.drop);
        if (typeof outcome === "object") {
          setPending({ action, presses: presses.slice(i), details: outcome.confirm });
          return;
        }
        if (outcome === "failed") break;
      }
      setPending(null);
    } finally {
      setBusy(null);
    }
  };
  return {
    busy,
    pending,
    form,
    closeForm: () => setForm(null),
    run: (action: LensAction, row: Record<string, SqlCell> | null) => {
      if (action.form) setForm({ action, row });
      else void press(action, [{ row, drop: null }], false);
    },
    /** A bulk action on the marked rows, once each, in order. */
    runMany: (action: LensAction, rows: Record<string, SqlCell>[]) => void press(action, rows.map((row) => ({ row, drop: null })), false),
    /** A drag action for each dropped row, with where it landed. */
    runDrops: (action: LensAction, presses: Press[]) => void press(action, presses, false),
    confirm: () => {
      if (pending) void press(pending.action, pending.presses, true);
    },
    cancel: () => setPending(null),
  };
}

export type LensActionsState = ReturnType<typeof useLensActions>;

/** The toolbar's actions as the page's: Copy, Add to Agent Context and
 *  the whole-lens actions, handed up whenever what they say changes. */
function HandToolbar({ run, streamId, actions, marks, to }: { run: LensRun; streamId: string | null; actions: LensActionsState; marks: Marks; to(actions: PageAction[]): void }) {
  const [copied, setCopied] = useState(false);
  const buttons = run.lens.actions.filter((a) => offeredWholeLens(a, run.result));
  const toRef = useRef(to);
  toRef.current = to;
  const pressRef = useRef(actions.run);
  pressRef.current = actions.run;
  const manyRef = useRef(actions.runMany);
  manyRef.current = actions.runMany;
  const marksRef = useRef(marks);
  marksRef.current = marks;
  const marked = markedRecords(run, marks);
  const markedKey = marks.keys.join("\u0000");
  useEffect(() => {
    // While rows are marked, the page's actions are what runs on them.
    if (marked.length > 0) {
      toRef.current([
        ...bulkOffers(run, marked).map(({ action, rows }) => ({
          id: `lens-bulk-${action.id}`,
          label: `${action.label} (${rows.length})`,
          title: action.command,
          busy: actions.busy === action.id,
          disabled: actions.busy !== null && actions.busy !== action.id,
          run: () => manyRef.current(action, rows),
        })),
        { id: "lens-marks-clear", label: `Clear selection (${marked.length})`, run: () => marksRef.current.set([], null) },
      ]);
      return;
    }
    toRef.current([
      {
        id: "lens-copy",
        label: copied ? "Copied" : "Copy",
        title: "Copy what the lens shows, as text",
        run: () =>
          void copyLens(run, streamId, {
            lensText,
            copyText: (t) => navigator.clipboard.writeText(t),
            recordError: (label, message) => recordOpError({ label, message }),
          }).then((ok) => {
            if (ok) {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            }
          }),
      },
      { id: "lens-add-to-context", label: "Add to Agent Context", run: () => addLensToContext(run, insertIntoAgent) },
      ...buttons.map((a) => ({
        id: `lens-action-${a.id}`,
        label: a.label,
        title: a.command,
        busy: actions.busy === a.id,
        disabled: actions.busy !== null && actions.busy !== a.id,
        run: () => pressRef.current(a, null),
      })),
    ]);
    // `buttons` and `marked` follow `run` and `markedKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run, streamId, copied, actions.busy, markedKey]);
  return null;
}

/** A part's whole-lens actions as buttons: no Copy, no marks. An editor's
 *  Save is the editor's own. */
function PartButtons({ run, actions }: { run: LensRun; actions: LensActionsState }) {
  const buttons = run.lens.actions.filter((a) => offeredWholeLens(a, run.result));
  if (buttons.length === 0) return null;
  return (
    <div data-testid="lens-part-actions" style={{ display: "flex", gap: 6, marginBottom: 6 }}>
      {buttons.map((a) => (
        <button key={a.id} type="button" data-testid={`lens-action-${a.id}`} disabled={actions.busy !== null} title={a.command} onClick={() => actions.run(a, null)}>
          {actions.busy === a.id ? `${a.label}…` : a.label}
        </button>
      ))}
    </div>
  );
}

/** Copy, Add to Agent Context, and the lens's whole-lens actions (a row's
 *  are in its menu, a group's in its heading). */
function LensToolbar({ run, streamId, actions, marks }: { run: LensRun; streamId: string | null; actions: LensActionsState; marks: Marks }) {
  const [copied, setCopied] = useState(false);
  const buttons = run.lens.actions.filter((a) => offeredWholeLens(a, run.result));
  if (markedRecords(run, marks).length > 0) return <MarksBar run={run} actions={actions} marks={marks} />;
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

function LensBody(props: LensResultViewProps & { actions?: LensActionsState; marks?: Marks; tree?: LazyTree | null }) {
  const { run, onOpenPage, streamId = null } = props;
  // It needs a capability that isn't active: it didn't run, and says what
  // it needs rather than showing nothing.
  if (run.inactive) {
    return (
      <p data-testid="lens-inactive" style={{ color: "var(--text-secondary)" }}>
        {run.inactive.message}
      </p>
    );
  }
  // A document or dashboard composes other lenses; its query, if any, is
  // one row its parts are given (`partParams`). Each is its own component
  // so the row views' hooks always run in the same order.
  if (run.lens.viz === "document") {
    return <DocumentViz parts={run.lens.parts} params={run.params} given={partParams(run)} streamId={streamId} onOpenPage={onOpenPage} onSetParam={props.onSetParam} />;
  }
  if (run.lens.viz === "dashboard") {
    return <DashboardViz run={run} actions={props.actions} parts={run.lens.parts} params={run.params} given={partParams(run)} streamId={streamId} onOpenPage={onOpenPage} onSetParam={props.onSetParam} />;
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
  actions,
  onCustomFailure,
  onCustomReady,
  componentView = null,
  onSetParam,
  marks,
  tree = null,
}: LensResultViewProps & { actions?: LensActionsState; marks?: Marks; tree?: LazyTree | null }) {
  const lens = run.lens;
  const limited = limitRows(run.result, maxRows);
  // The rows under a parent folded away (a `depth` lens: an epic's
  // children), kept in this step's history state like the groups'.
  const rowsClosed = useCollapsedSet("lens-rows.closed");
  const rowId = (row: SqlCell[]) => {
    const at = limited.columns.indexOf("uri");
    return at >= 0 && typeof row[at] === "string" ? (row[at] as string) : rowKey(row);
  };
  const folding = foldedAway(run.lens, limited.columns, limited.rows, new Set(rowsClosed.ids), rowId);
  const result = folding.hidden.size === 0 ? limited : { ...limited, rows: limited.rows.filter((r) => !folding.hidden.has(r)) };
  const picks = rowPicks(run, onSetParam);
  const ctxMenu = useContextMenu();
  const refOffersFor = useRefOffers();
  const extensions = useExtensions();
  const pageNav = useOptionalPageNavigation();
  // The groups folded away, and the rows opened into their `expand` lens
  // (both this step's history state).
  const folded = useCollapsedSet("lens-groups.closed");
  const expanded = useCollapsedSet("lens-rows.expanded");
  // A drag is taken only by the view its rows came from; where one would
  // land shows on the row (or group) under it.
  const dragToken = useId();
  // Where a Shift+click range starts before anything is marked: the last
  // row clicked (kept here, so a plain click writes no history).
  const clickedRef = useRef<number | null>(null);
  const [dropHint, setDropHint] = useState<{ key: string; zone: string } | null>(null);
  // The rows in drawing order and the cursor over them: ↑/↓ move it, Enter
  // opens the row's link or runs its first row action (`useListNav`).
  let drawn: SqlCell[][] = result.rows;
  let openRow: (row: SqlCell[]) => void = () => {};
  const rowNav = useListNav({ count: result.rows.length, onPick: (i) => drawn[i] && openRow(drawn[i]!), id: `lens-rows-${lens.id}` });
  // Extensions' badges for the refs these rows link to (P6b.C5).
  const linkedRefs = result.rows.flatMap((row) =>
    lens.columns.flatMap((c) => {
      const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
      return ref ? [linkTargetId(ref)] : [];
    }),
  );
  const badges = useDecorations("row-badge", linkedRefs);
  if (result.rows.length === 0) {
    // `empty: ""` is nothing to say (a part that only stands in for what an
    // extension may bring).
    if (lens.empty === "") return null;
    return (
      <p data-testid="lens-empty" style={{ color: "var(--text-secondary)" }}>
        {lens.empty ?? "No rows."}
      </p>
    );
  }
  const cols = displayColumns(lens, result.columns);
  // A link column's pages are one collection: Up and Down on the page a
  // row opens step through its siblings.
  const origins = new Map(
    cols.filter((c) => c.link).map((c) => [c.key, linkOrigins(lens.title, c, result.rows, result.columns)] as const),
  );
  // An edit action is the markdown editor's Save, not a row's.
  const rowActions0 = lens.actions.filter((a) => a.row && !a.edit);
  openRow = (row) => {
    const record = rowRecord(result.columns, row);
    const primary = rowActions0.find((a) => a.primary && actionOffered(a, record));
    if (primary) {
      actions?.run(primary, record);
      return;
    }
    for (const c of cols) {
      const target = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
      if (target !== null) {
        routeDispatch(target, pageNav, { onNavigate: tabNavigate(target, onOpenPage), origin: origins.get(c.key)?.(row) })(false);
        return;
      }
    }
    const action = rowActions0.find((a) => actionOffered(a, record));
    if (action) actions?.run(action, record);
  };
  const cell: CellRenderer = (row, c) => {
    const raw = row[c.index] ?? null;
    const formatted = formatCell(raw, c.unitIndex === null ? undefined : (row[c.unitIndex] ?? null), c.format);
    // A timestamp keeps its full local time a hover away.
    // A highlighted column marks its param's current value in the text.
    const needle = c.highlight === null ? "" : String(run.params[c.highlight] ?? "");
    const series = c.format === "sparkline" ? sparklineValues(raw) : null;
    const text = series ? (
      <span data-testid="lens-sparkline" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
        <Sparkline values={series} width={72} height={18} />
        <span>{formatted}</span>
      </span>
    ) : c.format === "badge" && formatted !== "" ? (
      <span
        data-testid="lens-badge"
        className="ox-badge"
        style={toneColor(c.toneIndex === null ? null : (row[c.toneIndex] ?? null)) ? { color: toneColor(row[c.toneIndex!] ?? null)! } : undefined}
      >
        {formatted}
      </span>
    ) : isTimestamp(raw) ? (
      <span title={formatFullDateTime(raw)}>{formatted}</span>
    ) : needle !== "" && typeof formatted === "string" ? (
      markParts(formatted, needle).map((part, i) =>
        part.mark ? (
          <mark key={i} className="ox-mark">
            {part.text}
          </mark>
        ) : (
          part.text
        ),
      )
    ) : (
      formatted
    );
    // Its icon and tone, named by other columns of the row.
    const toneName = c.toneIndex === null ? null : (row[c.toneIndex] ?? null);
    const tone = toneColor(toneName);
    const icon = c.iconIndex === null ? null : lensIcon(row[c.iconIndex] ?? null, extensions);
    const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
    const mine = ref ? badges.filter((b) => b.ref === linkTargetId(ref)) : [];
    const body = ref ? (
      <RouteLink to={ref} onNavigate={tabNavigate(ref, onOpenPage)} origin={origins.get(c.key)?.(row)} style={tone ? { ...linkStyle, color: tone } : linkStyle}>
        {text}
      </RouteLink>
    ) : (
      text
    );
    const opens = c.expand ? expanded.has(expansionKey(c, row)) : false;
    if (!tone && !icon && mine.length === 0 && !c.expand) return body;
    return (
      <span
        data-tone={tone ? String(toneName) : undefined}
        style={{ display: "inline-flex", alignItems: "center", gap: 4, flexWrap: "wrap", color: tone ?? undefined }}
      >
        {c.expand ? (
          <button
            type="button"
            aria-expanded={opens}
            aria-label={opens ? "Collapse" : "Expand"}
            onClick={() => expanded.toggle(expansionKey(c, row))}
            style={disclosureStyle}
          >
            {opens ? "▾" : "▸"}
          </button>
        ) : null}
        {icon ? <LensIcon spec={icon} tone={tone} /> : null}
        {body}
        {mine.map((b, i) => (
          <RefBadge key={i} label={b.label} tone="label" color={safeColor(b.color) ?? undefined} title={`from ${b.extension}`} />
        ))}
      </span>
    );
  };
  const first = result.rows[0]?.[0] ?? null;
  const rowActions = lens.actions.filter((a) => a.row && !a.edit);
  const rowItems = (row: SqlCell[]) => [
    {
      id: "ask-about-row",
      label: "Ask About This",
      enabled: true,
      run: () => insertIntoAgent(rowAsk(lens, result.columns, row)),
    },
    ...rowActions
      .filter((a) => actionOffered(a, rowRecord(result.columns, row)))
      .map((a) => ({
      id: `lens-action-${a.id}`,
      label: a.label,
      enabled: actions !== undefined,
      run: () => actions?.run(a, rowRecord(result.columns, row)),
    })),
    // The commands about what the row links to (their `ui.about`).
    ...(() => {
      const ref = rowRef(lens, result.columns, row);
      return ref ? refCommandMenuItems(refOffersFor(ref)) : [];
    })(),
  ];
  // Every row of every row component: focusable, with its menu from a
  // right-click or the keyboard (Menu key / Shift+F10).
  // A row that picks: a click (not on a link or control in it) or Enter
  // sets the param its column selects.
  // The row action a click or Enter runs: the lens's `primary`, where the row is offered it.
  const primaryOf = (row: SqlCell[]) => {
    const record = rowRecord(result.columns, row);
    return rowActions.find((a) => a.primary && actionOffered(a, record)) ?? null;
  };
  const rowMenu: RowMenu = (row) => ({
    tabIndex: 0,
    // In the capture phase: a link in the row opens its page in a new tab
    // on its own right-click, and the row's menu is what a right-click on
    // a row means, wherever on it.
    onContextMenuCapture: (e) => ctxMenu.open(e, rowItems(row)),
    onContextMenu: (e) => ctxMenu.open(e, rowItems(row)),
    onKeyDown: (e) => {
      if (picks && e.key === "Enter" && e.target === e.currentTarget) {
        e.preventDefault();
        picks.pick(row);
        return;
      }
      const primary = primaryOf(row);
      if (primary && e.key === "Enter" && e.target === e.currentTarget) {
        e.preventDefault();
        actions?.run(primary, rowRecord(result.columns, row));
        return;
      }
      ctxMenu.openForKey(e, rowItems(row));
    },
    // Cmd/Ctrl+click marks the row, Shift+click the range to it; a plain
    // click clears the marks, then picks or runs the primary action.
    onClick: (e: React.MouseEvent) => {
      const index = drawnKeys.indexOf(rowKey(row));
      if (marks && (e.metaKey || e.ctrlKey || e.shiftKey)) {
        e.preventDefault();
        const anchor = marks.anchor ?? clickedRef.current;
        const next = marksAfterClick(marks.keys, drawnKeys, index, anchor, { toggle: e.metaKey || e.ctrlKey, range: e.shiftKey });
        marks.set(next.marks, next.anchor);
        return;
      }
      if ((e.target as Element).closest("a, button, input, select, textarea")) return;
      clickedRef.current = index;
      if (marks && marks.keys.length > 0) marks.set([], null);
      if (picks) {
        picks.pick(row);
        return;
      }
      const primary = primaryOf(row);
      if (primary) actions?.run(primary, rowRecord(result.columns, row));
    },
    ...(picks ? { "aria-selected": picks.selected(row) } : {}),
  });
  // A grouped list or table: its headings, each with its group's actions.
  const groups = rowGroups(lens, result);
  // A tree draws its rows in nesting order (a lazy tree's spliced rows too).
  const treeNodesDrawn = lens.viz === "tree" ? (tree?.nodes ?? treeNodes(lens, result)) : null;
  drawn = groups ? groups.filter((g) => !folded.has(g.key)).flatMap((g) => g.rows) : treeNodesDrawn ? drawnTreeRows(treeNodesDrawn) : result.rows;
  const drawnAt = new Map(drawn.map((r, i) => [r, i] as const));
  // Marks, drags and keys (`rowInteract.ts`).
  const drawnKeys = drawn.map(rowKey);
  const marked = marks ? shownMarks(marks.keys, drawnKeys) : [];
  const markedSet = new Set(marked);
  const recordOf = (row: SqlCell[]) => rowRecord(result.columns, row);
  const dragActions = lens.actions.filter((a) => a.drag);
  const onto = takesOnto(dragActions);
  const byIndex = lens.group ? result.columns.indexOf(lens.group.by) : -1;
  const groupOf = (row: SqlCell[]): SqlCell => (byIndex === -1 ? null : (row[byIndex] ?? null));
  const sameGroup = (a: SqlCell[], b: SqlCell[]) => byIndex === -1 || groupOf(a) === groupOf(b);
  // A marked row drags every marked row with it.
  const draggedWith = (row: SqlCell[]) => (markedSet.has(rowKey(row)) ? drawn.filter((r) => markedSet.has(rowKey(r))) : [row]);
  const carriesRows = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes(LENS_ROW_DRAG_MIME);
  const readDrag = (e: React.DragEvent): SqlCell[][] | null => {
    try {
      const p = JSON.parse(e.dataTransfer.getData(LENS_ROW_DRAG_MIME)) as { token?: unknown; keys?: unknown };
      if (p.token !== dragToken || !Array.isArray(p.keys)) return null;
      const keys = new Set(p.keys as string[]);
      return drawn.filter((r) => keys.has(rowKey(r)));
    } catch {
      return null;
    }
  };
  // The drag action the drop runs, once per dragged row, each landing
  // after the one before it; its order is every row of the result.
  const dropRows = (dragged: SqlCell[][], place: DropAt) => {
    const records = dragged.map(recordOf);
    const at = withOrder(place, result.rows.map(recordOf), records);
    const first = records[0];
    const action = first ? dragActionFor(dragActions, first, at) : null;
    if (!action || !actions) return;
    actions.runDrops(action, chainedDrops(at, records).map((drop, i) => ({ row: records[i]!, drop })));
  };
  const rowDrop = (row: SqlCell[]): Record<string, unknown> => {
    if (dragActions.length === 0) return {};
    const key = rowKey(row);
    const zoneOf = (e: React.DragEvent) => {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      return dropZone(e.clientY, rect.top, rect.height, onto);
    };
    return {
      "data-drop": dropHint?.key === key ? dropHint.zone : undefined,
      onDragOver: (e: React.DragEvent) => {
        if (!carriesRows(e)) return;
        e.preventDefault();
        e.stopPropagation();
        const zone = zoneOf(e);
        if (dropHint?.key !== key || dropHint.zone !== zone) setDropHint({ key, zone });
      },
      onDragLeave: () => setDropHint((h) => (h?.key === key ? null : h)),
      onDrop: (e: React.DragEvent) => {
        if (!carriesRows(e)) return;
        e.preventDefault();
        e.stopPropagation();
        setDropHint(null);
        const dragged = readDrag(e);
        if (!dragged || dragged.some((r) => rowKey(r) === key)) return;
        const away = new Set(dragged.map(rowKey));
        const list = drawn.filter((r) => sameGroup(r, row) && !away.has(rowKey(r)));
        dropRows(dragged, dropBeside(list.map(recordOf), list.indexOf(row), zoneOf(e), groupOf(row)));
      },
    };
  };
  // A group's heading, or a board column: a drop there lands at its end.
  const groupDrop = (g: RowGroup): Record<string, unknown> => {
    if (dragActions.length === 0) return {};
    const key = `group:${g.key}`;
    return {
      "data-drop": dropHint?.key === key ? "in" : undefined,
      onDragOver: (e: React.DragEvent) => {
        if (!carriesRows(e)) return;
        e.preventDefault();
        if (dropHint?.key !== key) setDropHint({ key, zone: "in" });
      },
      onDragLeave: (e: React.DragEvent) => {
        if ((e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) return;
        setDropHint((h) => (h?.key === key ? null : h));
      },
      onDrop: (e: React.DragEvent) => {
        if (!carriesRows(e)) return;
        e.preventDefault();
        e.stopPropagation();
        setDropHint(null);
        const dragged = readDrag(e);
        if (!dragged) return;
        const away = new Set(dragged.map(rowKey));
        dropRows(dragged, dropInGroup(g.rows.filter((r) => !away.has(rowKey(r))).map(recordOf), g.value));
      },
    };
  };
  // A list, table or tree row: dragged where a drag action runs, or into
  // the agent's context when it links somewhere; marked; highlighted by
  // `emphasis`, indented by `depth`.
  const rowLook: RowLook = (row) => {
    const ref = rowContextRef(lens, result.columns, row);
    const emphasized = rowEmphasized(lens, result.columns, row);
    const drags = dragActions.some((a) => actionOffered(a, recordOf(row)));
    return {
      props: {
        ...(ref || drags
          ? {
              draggable: true,
              onDragStart: (e: React.DragEvent) => {
                if (ref) setContextRefDrag(e, ref);
                if (drags) {
                  e.dataTransfer.setData(LENS_ROW_DRAG_MIME, JSON.stringify({ token: dragToken, keys: draggedWith(row).map(rowKey) }));
                  e.dataTransfer.effectAllowed = ref ? "copyMove" : "move";
                }
              },
              onDragEnd: () => setDropHint(null),
            }
          : {}),
        ...rowDrop(row),
        ...(markedSet.has(rowKey(row)) ? { "data-marked": "true" } : {}),
        ...(emphasized ? { "data-emphasis": "true" } : {}),
      },
      emphasized,
      indent: rowDepth(lens, result.columns, row) * DEPTH_INDENT,
      fold: folding.parents.has(row) ? { open: !rowsClosed.has(rowId(row)), toggle: () => rowsClosed.toggle(rowId(row)) } : null,
      picks: picks !== null,
      selected: picks?.selected(row) ?? false,
    };
  };
  const heading = (g: RowGroup) => (
    <div {...groupDrop(g)}>
    <GroupHeading
      group={g}
      open={!folded.has(g.key)}
      onToggle={() => folded.toggle(g.key)}
      actions={lens.actions.filter((a) => a.group === g.key)}
      busy={actions?.busy ?? null}
      onAction={(a) => actions?.run(a, null)}
      onOpenPage={onOpenPage}
    />
    </div>
  );
  // Under a row: the lens each of its open `expand` columns names, run
  // with the row's values.
  const detail = (row: SqlCell[], index: number): ReactNode => {
    const open = cols.filter((c) => c.expand && expanded.has(expansionKey(c, row)));
    if (open.length === 0) return null;
    return (
      <div data-testid={`lens-row-expansion-${index}`} style={expansionStyle}>
        {open.map((c) => (
          <RowExpansion key={c.key} lensId={c.expand!} params={rowRecord(result.columns, row)} streamId={streamId} onOpenPage={onOpenPage} />
        ))}
      </div>
    );
  };
  const rowsView: RowsView = {
    groups,
    folded: (g) => folded.has(g.key),
    detail,
    heading,
    groupDrop,
    rowLook,
    cursor: {
      // The list takes the keys itself: a row's own (Enter that picks, the
      // menu key) and a control's (an input in a cell) go first.
      listProps: {
        tabIndex: 0,
        onKeyDown: (e) => {
          if (e.defaultPrevented || (e.target as Element).closest("input, select, textarea")) return;
          const at = drawn[rowNav.cursor];
          // Space marks the cursor row; Escape clears the marks.
          if (marks && e.key === " " && at) {
            e.preventDefault();
            const next = marksAfterClick(marks.keys, drawnKeys, rowNav.cursor, marks.anchor, { toggle: true, range: false });
            marks.set(next.marks, next.anchor);
            return;
          }
          if (marks && e.key === "Escape" && marked.length > 0) {
            e.preventDefault();
            e.stopPropagation();
            marks.set([], marks.anchor);
            return;
          }
          // A row action's key: on the marked rows when it is a bulk one
          // and some are marked, else on the cursor row.
          const keyed = e.metaKey || e.ctrlKey || e.altKey ? null : keyAction(lens.actions, e.key);
          if (keyed && actions) {
            e.preventDefault();
            const targets = keyed.bulk && marked.length > 0 ? drawn.filter((r) => markedSet.has(rowKey(r))) : at ? [at] : [];
            const records = targets.map(recordOf).filter((r) => actionOffered(keyed, r));
            if (records.length > 0) actions.runMany(keyed, records);
            return;
          }
          rowNav.keyProps.onKeyDown(e);
        },
      },
      isCursor: (row) => drawnAt.get(row) === rowNav.cursor,
      point: (row) => rowNav.setCursor(drawnAt.get(row) ?? 0),
    },
  };
  return (
    <>
      {compact ? null : <ResultCompleteness result={result} />}
      <LensViz {...{ run, result, lens, cols, cell, first, compact, streamId, onOpenPage, rowMenu, rowsView, onCustomFailure, onCustomReady, componentView }} tree={treeNodesDrawn ? { nodes: treeNodesDrawn, open: tree?.open ?? null } : null} />
      {ctxMenu.menu}
    </>
  );
}

/** How far each `depth` level indents a row, px. */
const DEPTH_INDENT = 14;

interface RowLookOut {
  props: Record<string, unknown>;
  emphasized: boolean;
  /** Extra left padding, px. */
  indent: number;
  /** A parent row folds its children away. */
  fold: { open: boolean; toggle(): void } | null;
  /** Clicking it sets a param; `selected`: it holds that param's value. */
  picks: boolean;
  selected: boolean;
}

/** The param a lens's rows set when clicked (its column's `select`), if
 *  it has one and there's somewhere to set it. */
function rowPicks(run: LensRun, onSetParam: LensResultViewProps["onSetParam"]) {
  const column = run.lens.columns.find((c) => c.select);
  const index = column ? run.result.columns.indexOf(column.key) : -1;
  if (!onSetParam || !column?.select || index === -1) return null;
  const name = column.select;
  const current = run.params[name] ?? null;
  const cols = displayColumns(run.lens, run.result.columns);
  // The row as the page's rail shows it: each column's label and text.
  const facts = (row: SqlCell[]) =>
    cols.map((c) => ({ label: c.label, text: formatCell(row[c.index] ?? null, c.unitIndex === null ? undefined : (row[c.unitIndex] ?? null), c.format) }));
  return {
    pick: (row: SqlCell[]) => onSetParam(name, row[index] ?? null, { value: row[index] ?? null, facts: facts(row) }),
    selected: (row: SqlCell[]) => current !== null && String(row[index] ?? "") === String(current),
  };
}
type RowLook = (row: SqlCell[]) => RowLookOut;

/** What a list, table or tree draws besides cells. */
interface RowsView {
  groups: RowGroup[] | null;
  /** Whether a group's rows are folded away under its heading. */
  folded(g: RowGroup): boolean;
  /** What a row opens under itself (its `expand` lenses), if anything. */
  detail(row: SqlCell[], index: number): ReactNode;
  heading(g: RowGroup): ReactNode;
  /** A drop target for a whole group (a board column). */
  groupDrop(g: RowGroup): Record<string, unknown>;
  rowLook: RowLook;
  /** The keyboard cursor over the drawn rows (list and table). */
  cursor: {
    listProps: React.HTMLAttributes<HTMLElement>;
    isCursor(row: SqlCell[]): boolean;
    point(row: SqlCell[]): void;
  };
}

/** A group's heading, styled as the rail's sub-headings: its value
 *  (linked when the group's `link` resolves) and its group actions. */
function GroupHeading({
  group,
  open,
  onToggle,
  actions,
  busy,
  onAction,
  onOpenPage,
}: {
  group: RowGroup;
  open: boolean;
  onToggle(): void;
  actions: LensAction[];
  busy: string | null;
  onAction(a: LensAction): void;
  onOpenPage?(ref: TabRef): void;
}) {
  const ref = group.ref;
  return (
    <div data-testid="lens-group-heading" data-group={group.key} style={groupHeadingStyle}>
      <button
        type="button"
        aria-expanded={open}
        aria-label={`${open ? "Collapse" : "Expand"} ${group.label}`}
        onClick={onToggle}
        style={disclosureStyle}
      >
        {open ? "▾" : "▸"}
      </button>
      {ref ? (
        <RouteLink
          to={ref}
          testId="lens-group-link"
          onNavigate={tabNavigate(ref, onOpenPage)}
          style={{ ...linkStyle, color: "inherit", letterSpacing: "inherit", textTransform: "inherit" }}
        >
          {group.label}
        </RouteLink>
      ) : (
        <span>{group.label}</span>
      )}
      <span data-testid="lens-group-count" style={{ fontWeight: "normal", color: "var(--text-muted)" }}>
        {group.rows.length}
      </span>
      {actions.length > 0 ? (
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 4 }}>
          {actions.map((a) => (
            <button
              key={a.id}
              type="button"
              data-testid={`lens-group-action-${a.id}`}
              disabled={busy !== null}
              title={a.label}
              aria-label={a.label}
              onClick={() => onAction(a)}
              style={groupActionStyle}
            >
              {busy === a.id ? `${a.label}…` : a.label}
            </button>
          ))}
        </span>
      ) : null}
    </div>
  );
}

type RowMenu = (row: SqlCell[]) => {
  tabIndex: number;
  onContextMenuCapture(e: React.MouseEvent): void;
  onContextMenu(e: React.MouseEvent): void;
  onKeyDown(e: React.KeyboardEvent): void;
  onClick?(e: React.MouseEvent): void;
  "aria-selected"?: boolean;
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
  rowsView,
  onCustomFailure,
  onCustomReady,
  componentView,
  tree,
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
  /** A tree's nodes, and the open set a lazy tree's branches toggle. */
  tree: { nodes: TreeNode[]; open: CollapsedSet | null } | null;
  rowMenu: RowMenu;
  rowsView: RowsView;
  onCustomFailure?(reason: string): void;
  onCustomReady?(): void;
  componentView: HostedView | null;
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
      return <MarkdownViz run={{ ...run, result }} streamId={streamId} />;
    case "tree":
      return (
        <TreeViz
          nodes={tree!.nodes}
          lazyOpen={tree!.open}
          problem={lazyTreeProblem(lens, result)}
          label={treeLabelColumn(lens, cols, result.columns)}
          cell={cell}
          rowMenu={rowMenu}
          rowLook={rowsView.rowLook}
        />
      );
    case "timeline":
      return <TimelineViz entries={timelineEntries(lens, result)} onOpenPage={onOpenPage} rowMenu={rowMenu} />;
    case "detail":
      return <DetailViz row={result.rows[0]!} cols={cols} cell={cell} rowMenu={rowMenu} />;
    case "steps":
      return <StepsViz steps={stepItems(lens, result)} rowMenu={rowMenu} />;
    case "approvals":
      // The pending proposals are read by the kit component itself; the
      // lens's rows are ignored, its params narrow the list.
      return <Approvals thread={paramText(run.params.thread)} setting={paramText(run.params.setting)} />;
    case "hunks":
      return <HunksViz rows={hunkRows(lens, result)} streamId={streamId} rowMenu={rowMenu} />;
    case "code":
      return <CodeViz rows={codeRows(lens, result)} streamId={streamId} sync={lens.code?.sync ?? false} onOpenPage={onOpenPage} />;
    case "diff":
      return <DiffViz spec={diffRow(lens, result)} streamId={streamId} onOpenPage={onOpenPage} />;
    case "list":
      return <ListViz rows={result.rows} cols={cols} cell={cell} rowMenu={rowMenu} view={rowsView} />;
    case "board":
      return <BoardViz rows={result.rows} cols={cols} cell={cell} rowMenu={rowMenu} view={rowsView} />;
    case "custom":
      return (
        <CustomComponentViz
          run={{ ...run, result }}
          streamId={streamId}
          fallback={<TableViz rows={result.rows} cols={cols} cell={cell} rowMenu={rowMenu} />}
          onFailure={onCustomFailure}
          onReady={onCustomReady}
          view={componentView}
        />
      );
    case "table":
    default:
      return <TableViz rows={result.rows} cols={cols} cell={cell} rowMenu={rowMenu} view={rowsView} />;
  }
}

/** `form`: the fields of its command's input, starting from the form's
 *  values; submitting runs the command as the lens, for the person, then
 *  goes where the command says (`ui.open_after`) as the form's `then`
 *  says. */
function FormViz({ run, streamId }: { run: LensRun; streamId: string | null }) {
  const [start, setStart] = useState<FormStart | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  // The form's start is read again (not the form restarted) when what its
  // object fields were read from changes.
  const [reload, setReload] = useState(0);
  // What a second submit carries into the next entry (its `keep` fields).
  const [carried, setCarried] = useState<Record<string, unknown>>({});
  const againSubmitted = useRef<Record<string, unknown> | null>(null);
  // What the person typed is this step of the tab's (Back, Forward and a
  // restart give it back); null is untouched: the form shows the saved
  // values, whatever they are now. A secret is never in it.
  const [draft, setDraft, clearDraft] = useDraft<Drafts | null>(`form:${run.lens.id}`, null);
  const openAfter = useOpenAfter();
  const paramsKey = JSON.stringify(run.params);
  const again = run.lens.form?.again ?? null;
  const form = useFormSubmit(
    run.lens.title,
    (input, confirmed) => submitLensForm(run.lens.id, input, run.params, streamId, confirmed),
    (outcome) => {
      // A fresh form for the next entry, started again from the current
      // values: an edit form shows what it saved.
      clearDraft();
      setRound((r) => r + 1);
      const second = againSubmitted.current;
      againSubmitted.current = null;
      // The second submit stays: it is for the next entry.
      setCarried(second ?? {});
      if (start && !second) {
        void openAfterAction({ label: run.lens.title, command: start.command.id, then: run.lens.form?.then ?? "open" }, outcome, openAfter);
      }
    },
  );
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
  }, [run.lens.id, paramsKey, streamId, round, reload]);
  useRerunOnChange(start?.reads ?? NO_READS, () => setReload((n) => n + 1));
  const schema = useMemo(
    () => (start ? withSubfields(start.command.input_schema as Record<string, unknown>, start.subfields ?? {}) : null),
    [start],
  );
  if (error) return <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)" }}>{error}</div>;
  if (!start || !schema) return null;
  const mode = run.lens.form?.mode ?? "create";
  return (
    <div data-testid="lens-form">
      {form.pending ? (
        <CommandConfirm
          label={run.lens.title}
          command={start.command.id}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
          testIdPrefix="lens-form-confirm"
        />
      ) : null}
      <SchemaForm
        key={round}
        schema={schema}
        initial={{ ...(start.values as Record<string, unknown>), ...carried }}
        fields={start.fields ?? undefined}
        fixed={start.fixed}
        choices={start.choices}
        serverErrors={form.serverErrors}
        mode={mode}
        submitLabel={mode === "edit" ? undefined : run.lens.title}
        draft={draft}
        onDraft={setDraft}
        busy={form.busy}
        onSubmit={(input) => void form.submit(input, false)}
        again={
          again
            ? {
                label: again.label,
                onSubmit: (input) => {
                  againSubmitted.current = Object.fromEntries(again.keep.filter((k) => k in input).map((k) => [k, input[k]]));
                  void form.submit(input, false);
                },
              }
            : undefined
        }
        testIdPrefix="lens-form-fields"
      />
    </div>
  );
}

/** A form action's form (`form: true`): its command's fields starting
 *  from the action's bound input, its fixed fields shown but not editable;
 *  submitting runs it as the lens, for the person, and closes it. A form
 *  that is one text field is a prompt strip at the top of the lens
 *  (`promptField`); any other is a slideover. */
function ActionForm({
  run,
  streamId,
  pressed,
  onClose,
}: {
  run: LensRun;
  streamId: string | null;
  pressed: { action: LensAction; row: Record<string, SqlCell> | null };
  onClose(): void;
}) {
  const { action, row } = pressed;
  const [start, setStart] = useState<FormStart | null>(null);
  const openAfter = useOpenAfter();
  const form = useFormSubmit(
    action.label,
    (input, confirmed) => runLensAction(run.lens.id, action.id, run.params, row, streamId, confirmed, input),
    (outcome) => {
      onClose();
      void openAfterAction(action, outcome, openAfter);
    },
  );
  const rowKey = JSON.stringify(row);
  useEffect(() => {
    let live = true;
    lensActionForm(run.lens.id, action.id, run.params, row, streamId)
      .then((s) => {
        if (live) setStart(s);
      })
      .catch((e) => {
        recordOpError({ label: action.label, message: e instanceof Error ? e.message : String(e) });
        onClose();
      });
    return () => {
      live = false;
    };
    // rowKey stands in for `row`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run.lens.id, action.id, rowKey, streamId]);
  // Which shape opens is the form's to say: nothing until it is read.
  if (!start) return null;
  const submitLabel = action.label.replace(/…$/, "");
  const field = promptField(start);
  if (field) {
    return <ActionPrompt label={action.label} start={start} field={field} form={form} submitLabel={submitLabel} onClose={onClose} testId="lens-action-form" />;
  }
  return <FormSlideover title={action.label} start={start} form={form} submitLabel={submitLabel} onClose={onClose} testId="lens-action-form" />;
}

/** A form that is one text field, as a prompt strip at the top of the
 *  lens (the Project panel's Rename… is the same strip): the field's
 *  description is its message, the bound value is selected to type over,
 *  Enter runs, Escape cancels. A secret field is masked, and blank leaves
 *  it out (it keeps what is saved). A command that asks is confirmed above it;
 *  a refusal shows under it and the strip stays with what was typed.
 *  `testId` is the slideover's, so whoever presses the action finds the
 *  field and the submit at the same ids whichever shape opens. */
function ActionPrompt({
  label,
  start,
  field,
  form,
  submitLabel,
  onClose,
  testId,
}: {
  label: string;
  start: FormStart;
  field: Field & { secret: boolean };
  form: ReturnType<typeof useFormSubmit>;
  submitLabel: string;
  onClose(): void;
  testId: string;
}) {
  const values = (start.values ?? {}) as Record<string, unknown>;
  // A secret is never required: blank keeps the saved one, so it is left out.
  const optional = !field.required || field.secret;
  const errors = form.serverErrors;
  const error = errors ? (errors[field.key] ?? Object.values(errors)[0] ?? null) : null;
  return (
    <div data-testid={testId} data-shape="prompt">
      {form.pending ? (
        <CommandConfirm
          label={label}
          command={start.command.id}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
          testIdPrefix={`${testId}-confirm`}
        />
      ) : null}
      <InlinePromptStrip
        message={field.description ?? field.label}
        fields={[{ key: field.key, initialValue: String(values[field.key] ?? ""), placeholder: field.label, optional, secret: field.secret }]}
        confirmLabel={submitLabel}
        busy={form.busy}
        error={error}
        onSubmit={(v) => void form.submit(field.secret && !v[field.key] ? {} : { [field.key]: v[field.key] }, false)}
        onCancel={onClose}
        testId={`${testId}-fields`}
      />
    </div>
  );
}

/** A command's form in a slideover: its fields starting from `start`'s
 *  values (its fixed fields shown, not editable), a command that asks
 *  confirmed in the host. `form` is the `useFormSubmit` that runs it;
 *  `testId` prefixes the slideover, its confirm and its fields. */
export function FormSlideover({
  title,
  start,
  form,
  submitLabel,
  onClose,
  testId,
}: {
  title: string;
  start: FormStart | null;
  form: ReturnType<typeof useFormSubmit>;
  submitLabel: string;
  onClose(): void;
  testId: string;
}) {
  return (
    <Slideover open onClose={onClose} title={title} testId={testId}>
      {form.pending && start ? (
        <CommandConfirm
          label={title}
          command={start.command.id}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
          testIdPrefix={`${testId}-confirm`}
        />
      ) : null}
      {start ? (
        <SchemaForm
          schema={start.command.input_schema as Record<string, unknown>}
          initial={start.values}
          fixed={start.fixed}
          choices={start.choices}
          fields={start.fields ?? undefined}
          serverErrors={form.serverErrors}
          submitLabel={submitLabel}
          busy={form.busy}
          onSubmit={(input) => void form.submit(input, false)}
          testIdPrefix={`${testId}-fields`}
        />
      ) : null}
    </Slideover>
  );
}

/** The column a tree's labels are drawn as: its declared column (so its
 *  link, icon and tone apply), else the plain `tree.label` column. */
function treeLabelColumn(lens: LensRun["lens"], cols: DisplayColumn[], resultColumns: string[]): DisplayColumn | null {
  const key = lens.tree?.label;
  if (!key) return null;
  const declared = cols.find((c) => c.key === key);
  if (declared) return declared;
  const index = resultColumns.indexOf(key);
  return index === -1 ? null : { key, label: key, index, link: null, unitIndex: null, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null };
}

interface TreeVizProps {
  label: DisplayColumn | null;
  cell: CellRenderer;
  rowMenu: RowMenu;
  rowLook: RowLook;
}

/** `tree`: nested rows, each branch collapsible. A branch the result
 *  nests is open by default (the closed ones, by index path, are this
 *  step's history state); a lazy branch (`tree.lazy`) is closed by default
 *  and opens by its id (`lazyOpen`), its row's click or Enter toggling it. */
function TreeViz({ nodes, lazyOpen, problem, ...rest }: TreeVizProps & { nodes: TreeNode[]; lazyOpen: CollapsedSet | null; problem: string | null }) {
  const closed = useCollapsedSet("lens-tree.closed");
  return (
    <>
      {problem ? (
        <div className="ox-error" data-testid="lens-tree-problem">
          {problem}
        </div>
      ) : null}
      <ul data-testid="lens-tree" style={treeListStyle}>
        {nodes.map((n, i) => (
          <TreeItem key={n.id ?? i} node={n} path={String(i)} closed={closed} lazyOpen={lazyOpen} {...rest} />
        ))}
      </ul>
    </>
  );
}

function TreeItem({ node, path, closed, lazyOpen, ...rest }: TreeVizProps & { node: TreeNode; path: string; closed: CollapsedSet; lazyOpen: CollapsedSet | null }) {
  const { label, cell, rowMenu, rowLook } = rest;
  const lazy = node.lazy && lazyOpen !== null && node.id !== null;
  const open = lazy ? lazyOpen.has(node.id!) : node.branch && !closed.has(path);
  const toggle = () => (lazy ? lazyOpen.toggle(node.id!) : closed.toggle(path));
  const look = rowLook(node.row);
  const menu = rowMenu(node.row);
  // A lazy branch's row opens on a plain click or Enter; a control or link
  // in it, and a marking click, keep their meaning.
  const props = lazy
    ? {
        ...menu,
        onClick: (e: React.MouseEvent) => {
          if (e.metaKey || e.ctrlKey || e.shiftKey || (e.target as Element).closest("a, button, input, select, textarea")) return menu.onClick?.(e);
          toggle();
        },
        onKeyDown: (e: React.KeyboardEvent) => {
          if (e.key === "Enter" && e.target === e.currentTarget) {
            e.preventDefault();
            toggle();
            return;
          }
          menu.onKeyDown(e);
        },
      }
    : menu;
  return (
    <li data-testid="lens-tree-node">
      <div
        data-testid="lens-tree-row"
        className={[look.picks || lazy ? "ox-clickable" : "", look.selected ? "ox-selected" : ""].filter(Boolean).join(" ") || undefined}
        style={{ display: "flex", alignItems: "center", gap: 4, padding: "2px 0", ...(look.emphasized || look.selected ? emphasisStyle : {}) }}
        {...props}
        {...look.props}
      >
        {node.branch ? (
          <button
            type="button"
            aria-expanded={open}
            aria-label={open ? "Collapse" : "Expand"}
            onClick={toggle}
            style={disclosureStyle}
          >
            {open ? "▾" : "▸"}
          </button>
        ) : (
          <span style={{ display: "inline-block", width: 16 }} />
        )}
        <span>{label ? cell(node.row, label) : node.label}</span>
      </div>
      {open && node.state === "loading" ? (
        <div data-testid="lens-tree-loading" style={{ ...treeNoteStyle }}>
          Loading…
        </div>
      ) : null}
      {open && node.state && node.state !== "loading" ? (
        <div className="ox-error" data-testid="lens-tree-error" style={treeNoteStyle}>
          {node.state.error}
        </div>
      ) : null}
      {node.children.length > 0 && open ? (
        <ul style={{ ...treeListStyle, paddingLeft: 16 }}>
          {node.children.map((c, i) => (
            <TreeItem key={c.id ?? i} node={c} path={`${path}.${i}`} closed={closed} lazyOpen={lazyOpen} {...rest} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

const treeNoteStyle: CSSProperties = { paddingLeft: 36, color: "var(--text-secondary)", fontSize: "var(--text-xs)" };

/** A row's `expand` column, as the set of opened ones names it. */
function expansionKey(c: DisplayColumn, row: SqlCell[]): string {
  return `${c.key}:${JSON.stringify(row)}`;
}

const expansionStyle: CSSProperties = { padding: "4px 0 8px 20px" };

/** An `expand` lens run for a row: the params it declares take the row's
 *  values, and its result is drawn in place. */
function RowExpansion({
  lensId,
  params,
  streamId,
  onOpenPage,
}: {
  lensId: string;
  params: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
}) {
  const runs = useLensRuns([{ key: "expansion", lensId }], params, streamId);
  const part = runs?.get("expansion");
  if (!part) return <p className="ox-muted ox-small">Loading…</p>;
  if (part.error) return <p className="ox-error">{part.error}</p>;
  return part.run ? <LensResultView run={part.run} compact toolbar={false} streamId={streamId} onOpenPage={onOpenPage} /> : null;
}

/** What a link's `onNavigate` is: a page built from the row goes to the
 *  host's opener when one is given; a ref string always opens through
 *  `openRef`. */
function tabNavigate(target: LinkTarget, onOpenPage?: (ref: TabRef) => void): (() => void) | undefined {
  return typeof target !== "string" && onOpenPage ? () => onOpenPage(target) : undefined;
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
        const ref = e.ref && parseUri(e.ref) ? e.ref : null;
        return (
          <li key={i} data-testid={`lens-timeline-entry-${i}`} style={{ padding: "4px 0 4px 12px" }} {...rowMenu(e.row)}>
            <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{e.at}</div>
            <div>
              {ref ? (
                <RouteLink to={ref} style={linkStyle}>
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
  return <Facts testId="lens-detail" facts={cols.map((c) => ({ key: c.key, label: c.label, value: cell(row, c) }))} {...rowMenu(row)} />;
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
  // At most one diff is open (each is an editor); it is this step's history state.
  const openSet = useCollapsedSet("lens-hunks.open", rows.length === 1 ? ["0"] : []);
  const open = openSet.ids.length > 0 ? Number(openSet.ids[0]) : null;
  const setOpen = (next: (o: number | null) => number | null) => {
    const to = next(open);
    openSet.set(to === null ? [] : [String(to)]);
  };
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


function LineViz({ run }: { run: LensRun }) {
  const series = lineSeries(run.lens, run.result);
  return (
    <div data-testid="lens-line">
      {series.map((s) => (
        <div key={s.name} data-testid="lens-line-series" style={{ marginBottom: 12 }}>
          {s.name ? <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{s.name}</div> : null}
          <TrendChart points={s.points} target={s.target} width={640} height={180} />
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
  const nav = useOptionalPageNavigation();
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
    if (typeof ref === "string") (nav?.openRef ?? openRefInWindow)(ref, {});
    else if (ref) onOpenPage?.(ref);
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

function NumberViz({ value, compact }: { value: SqlCell; compact: boolean }) {
  const title = typeof value === "number" ? formatMetricValueExact(value) : undefined;
  const text = typeof value === "number" ? formatMetricValue(value) : formatCell(value);
  // A compact strip shows it inline; anywhere else it is the place's figure.
  if (compact) {
    return (
      <span data-testid="lens-number" title={title} style={{ color: "var(--text-secondary)", fontWeight: 500 }}>
        {text}
      </span>
    );
  }
  return <Stat testId="lens-number" big value={text} title={title} />;
}

interface RowsVizProps {
  rows: SqlCell[][];
  cols: DisplayColumn[];
  cell: CellRenderer;
  /** Right-click on a row (per-row actions are right-click only). */
  rowMenu: RowMenu;
  /** Groups, emphasis, depth and dragging; absent, plain rows. */
  view?: RowsView;
}

/** The rows in drawing order, each with its index in the result (its
 *  test id), under their group's heading when the lens is grouped. */
function sections(rows: SqlCell[][], view: RowsView | undefined): { group: RowGroup | null; rows: [SqlCell[], number][] }[] {
  const index = new Map(rows.map((r, i) => [r, i] as const));
  const groups = view?.groups;
  if (!groups) return [{ group: null, rows: rows.map((r, i) => [r, i]) }];
  return groups.map((g) => ({ group: g, rows: view.folded(g) ? [] : g.rows.map((r) => [r, index.get(r) ?? 0]) }));
}

const noLook: RowLookOut = { props: {}, emphasized: false, indent: 0, fold: null, picks: false, selected: false };

function ListViz({ rows, cols, cell, rowMenu, view }: RowsVizProps) {
  const [head, ...rest] = cols;
  return (
    <>
      <ul data-testid="lens-list" style={{ listStyle: "none", padding: 0, margin: 0 }} {...(view?.cursor.listProps as React.HTMLAttributes<HTMLUListElement>)}>
        {sections(rows, view).map(({ group, rows: inGroup }) => (
          <Fragment key={group?.key ?? ""}>
            {group && view ? <li style={{ listStyle: "none" }}>{view.heading(group)}</li> : null}
            {inGroup.map(([row, i]) => {
              const look = view?.rowLook(row) ?? noLook;
              return (
                <li
                  key={i}
                  data-testid={`lens-row-${i}`}
                  data-cursor={view?.cursor.isCursor(row) ? "true" : undefined}
                  onMouseEnter={() => view?.cursor.point(row)}
                  style={{
                    ...listRowStyle,
                    ...(look.emphasized || look.selected ? emphasisStyle : {}),
                    ...(look.picks ? { cursor: "pointer" } : {}),
                    ...(look.indent ? { paddingLeft: look.indent } : {}),
                  }}
                  {...rowMenu(row)}
                  {...look.props}
                >
                  <div>
                    {look.fold ? (
                      <button
                        type="button"
                        data-testid={`lens-row-fold-${i}`}
                        aria-expanded={look.fold.open}
                        aria-label={look.fold.open ? "Fold its children away" : "Show its children"}
                        className="ox-link"
                        onClick={look.fold.toggle}
                      >
                        {look.fold.open ? "▾" : "▸"}
                      </button>
                    ) : null}
                    {head ? cell(row, head) : null}
                  </div>
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
                  {view?.detail(row, i)}
                </li>
              );
            })}
          </Fragment>
        ))}
      </ul>
    </>
  );
}

/** `board`: a column per group (its `values` first, empty ones too), its
 *  heading over its cards; a card is a list row — the same marks, menu,
 *  keys and drags — and a drop on a column's empty space lands at its
 *  end. */
function BoardViz({ rows, cols, cell, rowMenu, view }: RowsVizProps) {
  const [head, ...rest] = cols;
  const index = new Map(rows.map((r, i) => [r, i] as const));
  return (
    <div data-testid="lens-board" className="ox-board" {...(view?.cursor.listProps as React.HTMLAttributes<HTMLDivElement>)}>
      {(view?.groups ?? []).map((g) => (
        <section key={g.key} data-testid={`lens-board-column-${g.key}`} className="ox-board-column" {...view?.groupDrop(g)}>
          {view?.heading(g)}
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {(view?.folded(g) ? [] : g.rows).map((row) => {
              const i = index.get(row) ?? 0;
              const look = view?.rowLook(row) ?? noLook;
              return (
                <li
                  key={i}
                  data-testid={`lens-row-${i}`}
                  className="ox-card"
                  data-cursor={view?.cursor.isCursor(row) ? "true" : undefined}
                  onMouseEnter={() => view?.cursor.point(row)}
                  style={look.emphasized || look.selected ? emphasisStyle : undefined}
                  {...rowMenu(row)}
                  {...look.props}
                >
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
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}

function TableViz({ rows, cols, cell, rowMenu, view }: RowsVizProps) {
  // Each row with its index in the result: its test id.
  type Row = [SqlCell[], number];
  const columns: DataColumn<Row>[] = cols.map((c) => ({
    key: c.key,
    label: c.label,
    width: c.width ?? undefined,
    render: ([row]) => cell(row, c),
  }));
  const parts = sections(rows, view);
  return (
    <DataTable<Row>
      testId="lens-table"
      tableProps={view?.cursor.listProps as React.HTMLAttributes<HTMLTableElement>}
      columns={columns}
      groups={parts.map(({ group, rows: inGroup }) => ({
        key: group?.key ?? "",
        heading: group && view ? view.heading(group) : null,
        rows: inGroup,
      }))}
      rowKey={([, i]) => i}
      rowDetail={([row, i]) => view?.detail(row, i) ?? null}
      rowProps={([row, i]) => {
        const look = view?.rowLook(row) ?? noLook;
        return {
          "data-testid": `lens-row-${i}`,
          "data-cursor": view?.cursor.isCursor(row) ? "true" : undefined,
          onMouseEnter: () => view?.cursor.point(row),
          className:
            [look.emphasized ? "ox-emphasis" : "", look.picks ? "ox-clickable" : "", look.selected ? "ox-selected" : ""].filter(Boolean).join(" ") ||
            undefined,
          ...rowMenu(row),
          ...(look.props as RowProps),
        };
      }}
      cellStyle={([row], _c, j) => {
        const indent = j === 0 ? (view?.rowLook(row) ?? noLook).indent : 0;
        return indent ? { paddingLeft: `calc(var(--space-2) + ${indent}px)` } : undefined;
      }}
    />
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
const listRowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)" };
/** An `emphasis` row: the soft accent wash an active row gets. */
const emphasisStyle: CSSProperties = { background: "var(--accent-soft-bg)", fontWeight: 500 };
/** A group's heading: the rail's sub-heading look. */
const groupHeadingStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "10px 0 4px",
  fontSize: 11,
  fontWeight: 600,
  color: "var(--text-secondary)",
  textTransform: "uppercase",
  letterSpacing: 0.4,
};
const groupActionStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: "0 4px",
  color: "var(--text-muted)",
  cursor: "pointer",
  font: "inherit",
  fontSize: 12,
  textTransform: "none",
};
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-link, var(--accent))",
  cursor: "pointer",
  textAlign: "left",
  font: "inherit",
};

/** A lens param as text, or null when it is unset or empty. */
function paramText(value: SqlCell | undefined): string | null {
  return value === undefined || value === null || value === "" ? null : String(value);
}
