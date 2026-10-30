import { useCollapsedSet, useDraft, type CollapsedSet } from "../tabs/useViewParam.js";
import { Approvals } from "../kit/Approvals.js";
import type { PickedRow } from "../pages/railGroups.js";
import { Fragment, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { commandOpenAfter, undoCommand, type VizRun, type SqlCell } from "../api.js";
import { FormSourceContext, useFormSource, type FormSource } from "./formSource.js";
import { useVizPages } from "./vizPages.js";
import { VizActionRunnerContext, useVizActionRunner, type VizActionRunner } from "./vizActionRunner.js";
import { TabModal } from "../components/TabModal.js";
import { InlinePromptStrip } from "../components/InlinePromptStrip.js";
import type { FormStart } from "../tauri-bridge/generated/bindings.js";
import type { Drop, PageAction as DeclaredPageAction } from "../tauri-bridge/generated/bindings.js";
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
import { shownFields, withSubfields, type Drafts, type Field } from "../components/SchemaForm/schemaFormModel.js";
import { NO_READS, useRerunOnChange } from "./rerun.js";
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
  columnWrap,
  hunkRows,
  sectionRows,
  itemList,
  isTimestamp,
  markParts,
  limitRows,
  lineSeries,
  rowAssistantContext,
  rowContext,
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
  codeRows,
  diffRow, editorRow, embedRow,
  type BranchRun,
  type DisplayColumn,
  type RowGroup,
  type StepStatus,
  type TreeNode,
} from "./vizModel.js";
import { askAssistant } from "../components/Assistant/assistantSession.js";
import { addToContext } from "../contextBasket.js";
import { useContextMenu, type MenuItems } from "../components/useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { refCommandMenuItems, useRefOffers } from "../components/refCommands.js";
import { safeColor } from "../components/decorators.js";
import { useDecorations } from "../components/useDecorations.js";
import { RefBadge } from "../components/RefBadge.js";
import { CustomComponentViz } from "./CustomComponentViz.js";
import type { HostedView } from "./hostedView.js";
import { EditorViz } from "./EditorViz.js";
import { TerminalViz } from "./TerminalViz.js";
import { EmbedViz } from "./EmbedViz.js";
import { CodeViz } from "./CodeViz.js";
import { DiffViz } from "./DiffViz.js";
import { DataTable, type DataColumn, type RowProps } from "../kit/DataTable.js";
import { Facts } from "../kit/Facts.js";
import { ItemsCell } from "./ItemsCell.js";
import { OpenPageLink } from "../kit/OpenPageLink.js";
import { SectionsViz } from "./SectionsViz.js";
import { Stat } from "../kit/Stat.js";
import { recordOpError } from "../opErrors.js";
import { showToast } from "../components/toastStore.js";
import { actionDoes, actionOffered, offeredWholePage, openAfterAction, performVizAction, promptField, rowRecord, type ActionTarget } from "./vizActions.js";
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
import { VIZ_ROW_DRAG_MIME } from "../dragMimes.js";
import { VizIcon, vizIcon, toneColor } from "./vizIcons.js";
import { useExtensions } from "../extensionsStore.js";
import { setContextRefDrag } from "../agent-context-dnd.js";

type CellRenderer = (row: SqlCell[], col: DisplayColumn) => ReactNode;

/**
 * Renders a viz run's result in the viz's viz. Shared by the viz page,
 * slots and dashboard viz tiles; `maxRows` caps rows for compact views.
 * A `grid` runs its child viz nodes with this run's params, in `streamId`.
 * Every viz has Copy and Add to Context; its declared actions run
 * their commands as the viz (row actions from a row's right-click menu).
 */
export function VizView(props: VizViewProps) {
  const { run, streamId = null } = props;
  // A lazy tree (`tree.lazy`): the folders open by their id, this step's
  // history state; each runs the viz (a page file's node: its page) again
  // for its rows, which are then rows of the viz like the root's —
  // marked, dragged, counted.
  const pages = useVizPages();
  const useBranches = pages?.useBranches ?? noBranches;
  const lazyOpen = useCollapsedSet(`viz-tree.open:${run.config.id}`);
  const lazy = run.config.viz === "tree" && !!run.config.tree?.lazy;
  const branches = useBranches(run, lazy ? lazyOpen.ids : [], streamId, props.maxRows);
  // A branch that is a run of its own (a page's) takes its rows' presses.
  const baseRunner = useVizActionRunner();
  const baseForms = useFormSource();
  const routed = useMemo(() => routeBranches(baseRunner, baseForms, branches), [baseRunner, baseForms, branches]);
  return (
    <VizActionRunnerContext.Provider value={routed.runner}>
      <FormSourceContext.Provider value={routed.forms}>
        <VizViewBody {...props} lazyOpen={lazy ? lazyOpen : null} branches={branches} />
      </FormSourceContext.Provider>
    </VizActionRunnerContext.Provider>
  );
}

/** No page to read branches from (a viz drawn outside a page node, a
 *  test): a lazy tree's branches stay closed. */
function noBranches(): ReadonlyMap<string, BranchRun> {
  return NO_BRANCHES;
}

const NO_BRANCHES: ReadonlyMap<string, BranchRun> = new Map();

/** The runner and form source a viz's rows press with: a row of a branch
 *  that carries its own (a page branch's run) goes there, any other the
 *  base's. A branch's row is found by its values. */
function routeBranches(runner: VizActionRunner, forms: FormSource, branches: ReadonlyMap<string, BranchRun>): { runner: VizActionRunner; forms: FormSource } {
  const own = [...branches.values()].filter((b) => b.run && (b.runner || b.forms));
  if (own.length === 0) return { runner, forms };
  const branchOf = (row: Record<string, SqlCell> | null) =>
    row === null ? null : (own.find((b) => b.run!.result.rows.some((r) => sameRecord(rowRecord(b.run!.result.columns, r), row))) ?? null);
  return {
    runner: (viz, action, params, row, ...rest) => (branchOf(row)?.runner ?? runner)(viz, action, params, row, ...rest),
    forms: {
      start: forms.start,
      submit: forms.submit,
      actionStart: (target, action, row, streamId) => (branchOf(row)?.forms ?? forms).actionStart(target, action, row, streamId),
    },
  };
}

function sameRecord(a: Record<string, SqlCell>, b: Record<string, SqlCell>): boolean {
  const keys = Object.keys(b);
  return keys.length > 0 && keys.every((k) => (a[k] ?? null) === (b[k] ?? null));
}

function VizViewBody(props: VizViewProps & { lazyOpen: CollapsedSet | null; branches: ReadonlyMap<string, BranchRun> }) {
  const { run, streamId = null, compact = false, lazyOpen, branches } = props;
  const actions = useVizActions(run, streamId);
  const marks = useMarks(run);
  const tree: LazyTree | null = lazyOpen ? { nodes: lazyTreeNodes(run.config, run.result, new Set(lazyOpen.ids), branches), open: lazyOpen } : null;
  const shown = tree ? { ...run, result: { ...run.result, rows: drawnTreeRows(tree.nodes) } } : run;
  // A command's confirmation shows wherever the row is — an answer in the
  // strip, a grid child, a dashboard tile — not only where the toolbar is.
  const confirm = actions.pending ? (
    <CommandConfirm
      label={actions.pending.action.label}
      command={actions.pending.action.command ?? ""}
      details={actions.pending.details}
      onConfirm={actions.confirm}
      onCancel={actions.cancel}
      testIdPrefix="viz-action-confirm"
    />
  ) : null;
  // A form action's form: a slideover beside whatever pressed it, or a
  // prompt strip at the top of the viz when the form is one text field.
  const form = actions.form ? (
    <ActionForm run={run} streamId={streamId} pressed={actions.form} onClose={actions.closeForm} />
  ) : null;
  // Compact strips (a number inline) have no room for buttons; the page
  // draws Copy and its own actions in its nav bar.
  return (
    <>
      {confirm}
      {form}
      {!compact ? <MarksBar run={shown} actions={actions} marks={marks} /> : null}
      {props.partButtons ? <PartButtons run={shown} actions={actions} /> : null}
      <VizBody {...props} run={shown} tree={tree} actions={actions} marks={marks} />
    </>
  );
}

/** A lazy tree's nodes (`lazyTreeNodes`, its open branches' rows spliced
 *  in) and the open set they toggle. */
interface LazyTree {
  nodes: TreeNode[];
  open: CollapsedSet;
}

/** The rows a person has marked (`rowInteract.ts`): their keys in the
 *  entry's history state, per viz, so Back keeps them; and where a
 *  Shift+click range starts. */
interface Marks {
  keys: string[];
  anchor: number | null;
  set(keys: string[], anchor: number | null): void;
}

const NO_MARKS: { keys: string[]; anchor: number | null } = { keys: [], anchor: null };

function useMarks(run: VizRun): Marks {
  const [state, setState] = useHistoryState(`marked:${run.config.id}`, NO_MARKS);
  return { keys: state.keys, anchor: state.anchor, set: (keys, anchor) => setState({ keys, anchor }) };
}

/** The marked rows of `run`, in its order, as records. */
function markedRecords(run: VizRun, marks: Marks): Record<string, SqlCell>[] {
  const set = new Set(marks.keys);
  return run.result.rows.filter((r) => set.has(rowKey(r))).map((r) => rowRecord(run.result.columns, r));
}

/** The bulk actions (`bulk: true`) the marked rows are offered: an action
 *  any of them is offered (`when`), run on those. */
function bulkOffers(run: VizRun, marked: Record<string, SqlCell>[]): { action: DeclaredPageAction; rows: Record<string, SqlCell>[] }[] {
  return run.config.actions
    .filter((a) => a.bulk)
    .map((action) => ({ action, rows: marked.filter((r) => actionOffered(action, r)) }))
    .filter((o) => o.rows.length > 0);
}

/** While rows are marked: how many, the bulk actions, and Clear — in place
 *  of the viz's own buttons. */
function MarksBar({ run, actions, marks }: { run: VizRun; actions: VizActionsState; marks: Marks }) {
  const marked = markedRecords(run, marks);
  if (marked.length === 0) return null;
  return (
    <div data-testid="viz-marks" style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 6, marginBottom: 6 }}>
      <span data-testid="viz-marks-count" style={{ color: "var(--text-secondary)" }}>
        {marked.length} selected
      </span>
      {bulkOffers(run, marked).map(({ action, rows }) => (
        <button key={action.id} type="button" data-testid={`viz-bulk-${action.id}`} disabled={actions.busy !== null} title={actionDoes(action)} onClick={() => actions.runMany(action, rows)}>
          {actions.busy === action.id ? `${action.label}…` : action.label}
        </button>
      ))}
      <button type="button" data-testid="viz-marks-clear" onClick={() => marks.set([], null)}>
        Clear
      </button>
    </div>
  );
}

interface VizViewProps {
  run: VizRun;
  /** Where links go; without it they navigate through the route context. */
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  maxRows?: number;
  streamId?: string | null;
  /** Small inline rendering for strips (a number as plain text). */
  compact?: boolean;
  /** For a `viz: custom` viz whose component can't be shown: told why,
   *  instead of the viz showing its table (a replacement falls back to
   *  the core component, outside its own frame). */
  onCustomFailure?(reason: string): void;
  /** For a `viz: custom` viz: its component said `ready`. */
  onCustomReady?(): void;
  /** For a `viz: custom` viz: its host's view, where the host shares
   *  one with its component (a replacement, a viz's own page). */
  componentView?: HostedView | null;
  /** A `viz: custom` component's frame fits its content's height: a section of a document. */
  fitCustom?: boolean;
  /** Set one of the viz's params (a row click on a column that
   *  `select`s it, with the row it picked); without it, rows are no
   *  picks. */
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
  /** Its whole-viz actions (an Add…) as buttons above its body: a
   *  page's node does; a compact one has no room. */
  partButtons?: boolean;
  /** The viz is the page's own view (the whole of its body, not a snippet in
   *  another page, an embed or a row): what it shows is the page's resource,
   *  so it offers no link to open that resource. */
  isPage?: boolean;
}

/** Where a command's `ui.open_after` page opens from a viz: in the page
 *  the viz is on (in its place with `replace`), else the window. */
function useOpenAfter() {
  const nav = useOptionalPageNavigation();
  return {
    openAfter: commandOpenAfter,
    openPage: (page: string, replace: boolean) => void (nav?.openRef ?? openRefInWindow)(page, { replace }),
    closePage: nav?.close,
    recordError: (label: string, message: string) => recordOpError({ label, message }),
  };
}

/** Pressing a viz's actions, with the confirmation a command may ask
 *  for held until the person answers. */
/** One run of an action: the row it runs on, and where a dragged row
 *  landed. A bulk action or a drop of several rows is several. */
interface Press {
  row: Record<string, SqlCell> | null;
  drop: Drop | null;
}

/** `runner`: what the actions run through, when not the one the context
 *  gives (a page file's whole-page actions, pressed outside any node). */
export function useVizActions(run: ActionTarget, streamId: string | null, runner?: VizActionRunner) {
  const openAfter = useOpenAfter();
  const fromContext = useVizActionRunner();
  const pressOnRun = runner ?? fromContext;
  const [busy, setBusy] = useState<string | null>(null);
  // A command that asks: what it asks about, and the runs that wait on the
  // answer (one confirmation for a bulk action's runs).
  const [pending, setPending] = useState<{ action: DeclaredPageAction; presses: Press[]; details: string[] } | null>(null);
  // A form action pressed: its form is open until it runs or is closed.
  const [form, setForm] = useState<{ action: DeclaredPageAction; row: Record<string, SqlCell> | null } | null>(null);
  const deps = {
    press: (
      id: string,
      action: string,
      params: Record<string, SqlCell>,
      row: Record<string, SqlCell> | null,
      stream: string | null,
      confirmed: boolean,
      drop?: Drop | null,
    ) => pressOnRun(id, action, params, row, stream, confirmed, null, drop ?? null),
    toast: (message: string, undo?: () => void) => showToast({ message, onUndo: undo }),
    // The person's click on Undo is their confirmation of it.
    undo: (auditId: number) => undoCommand(auditId, true),
    ...openAfter,
  };
  // The runs in order; a run that asks stops them, the rest waiting on
  // the person's answer. Only the last says it's done (one toast).
  const press = async (action: DeclaredPageAction, presses: Press[], confirmed: boolean) => {
    setBusy(action.id);
    try {
      for (let i = 0; i < presses.length; i++) {
        const p = presses[i]!;
        const quiet = i < presses.length - 1 ? { ...deps, toast: () => {} } : deps;
        const outcome = await performVizAction(action, run, p.row, streamId, confirmed, quiet, p.drop);
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
    run: (action: DeclaredPageAction, row: Record<string, SqlCell> | null) => {
      if (action.form) setForm({ action, row });
      else void press(action, [{ row, drop: null }], false);
    },
    /** A bulk action on the marked rows, once each, in order. */
    runMany: (action: DeclaredPageAction, rows: Record<string, SqlCell>[]) => void press(action, rows.map((row) => ({ row, drop: null })), false),
    /** A drag action for each dropped row, with where it landed. */
    runDrops: (action: DeclaredPageAction, presses: Press[]) => void press(action, presses, false),
    confirm: () => {
      if (pending) void press(pending.action, pending.presses, true);
    },
    cancel: () => setPending(null),
  };
}

export type VizActionsState = ReturnType<typeof useVizActions>;

/** A part's whole-page actions as buttons: no Copy, no marks. An editor's
 *  Save is the editor's own. */
function PartButtons({ run, actions }: { run: VizRun; actions: VizActionsState }) {
  const buttons = run.config.actions.filter((a) => offeredWholePage(a, run.result));
  if (buttons.length === 0) return null;
  return (
    <div data-testid="viz-part-actions" style={{ display: "flex", gap: 6, marginBottom: 6 }}>
      {buttons.map((a) => (
        <button key={a.id} type="button" data-testid={`viz-action-${a.id}`} disabled={actions.busy !== null} title={actionDoes(a)} onClick={() => actions.run(a, null)}>
          {actions.busy === a.id ? `${a.label}…` : a.label}
        </button>
      ))}
    </div>
  );
}

function VizBody(props: VizViewProps & { actions?: VizActionsState; marks?: Marks; tree?: LazyTree | null }) {
  const { run, streamId = null } = props;
  if (run.config.viz === "form") return <FormViz run={run} streamId={streamId} />;
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
  fitCustom = false,
  onSetParam,
  marks,
  tree = null,
  isPage = false,
}: VizViewProps & { actions?: VizActionsState; marks?: Marks; tree?: LazyTree | null }) {
  const viz = run.config;
  const limited = limitRows(run.result, maxRows);
  // The rows under a parent folded away (a `depth` viz: an epic's
  // children), kept in this step's history state like the groups'.
  const rowsClosed = useCollapsedSet("viz-rows.closed");
  const rowId = (row: SqlCell[]) => {
    const at = limited.columns.indexOf("uri");
    return at >= 0 && typeof row[at] === "string" ? (row[at] as string) : rowKey(row);
  };
  const folding = foldedAway(run.config, limited.columns, limited.rows, new Set(rowsClosed.ids), rowId);
  const result = folding.hidden.size === 0 ? limited : { ...limited, rows: limited.rows.filter((r) => !folding.hidden.has(r)) };
  const picks = rowPicks(run, onSetParam);
  const ctxMenu = useContextMenu();
  const refOffersFor = useRefOffers();
  const extensions = useExtensions();
  const pageNav = useOptionalPageNavigation();
  // The groups folded away, and the rows opened into their `expand` viz
  // (both this step's history state).
  const folded = useCollapsedSet("viz-groups.closed");
  const expanded = useCollapsedSet("viz-rows.expanded");
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
  const rowNav = useListNav({ count: result.rows.length, onPick: (i) => drawn[i] && openRow(drawn[i]!), id: `viz-rows-${viz.id}` });
  // Extensions' badges for the refs these rows link to (P6b.C5).
  const linkedRefs = result.rows.flatMap((row) =>
    viz.columns.flatMap((c) => {
      const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
      return ref ? [linkTargetId(ref)] : [];
    }),
  );
  const badges = useDecorations("row-badge", linkedRefs);
  // A terminal viz with none is its own empty state: it opens the first.
  if (result.rows.length === 0 && viz.viz !== "terminal") {
    // `empty: ""` is nothing to say (a part that only stands in for what an
    // extension may bring).
    if (viz.empty === "") return null;
    return (
      <p data-testid="viz-empty" style={{ color: "var(--text-secondary)" }}>
        {viz.empty ?? "No rows."}
      </p>
    );
  }
  const cols = displayColumns(viz, result.columns);
  // A link column's pages are one collection: Up and Down on the page a
  // row opens step through its siblings.
  const origins = new Map(
    cols.filter((c) => c.link).map((c) => [c.key, linkOrigins(viz.title, { node: viz.id, params: run.params }, c, result.rows, result.columns)] as const),
  );
  // An edit action is the markdown editor's Save, not a row's.
  const rowActions0 = viz.actions.filter((a) => a.row && !a.edit);
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
    const itemsList = c.format === "items" ? itemList(raw) : null;
    const text = itemsList ? (
      <ItemsCell items={itemsList} linkStyle={linkStyle} />
    ) : series ? (
      <span data-testid="viz-sparkline" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
        <Sparkline values={series} width={72} height={18} />
        <span>{formatted}</span>
      </span>
    ) : c.format === "badge" && formatted !== "" ? (
      <span
        data-testid="viz-badge"
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
    const icon = c.iconIndex === null ? null : vizIcon(row[c.iconIndex] ?? null, extensions);
    // An empty value is no link: there is nothing there to open.
    const ref = c.link && raw !== null ? cellLinkRef(c.link, c.key, row, result.columns) : null;
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
        {icon ? <VizIcon spec={icon} tone={tone} /> : null}
        {body}
        {mine.map((b, i) => (
          <RefBadge key={i} label={b.label} tone="label" color={safeColor(b.color) ?? undefined} title={`from ${b.extension}`} />
        ))}
      </span>
    );
  };
  const first = result.rows[0]?.[0] ?? null;
  const rowActions = viz.actions.filter((a) => a.row && !a.edit);
  const rowItems = (row: SqlCell[]): MenuItems => {
    const base = rowBase(row);
    // The commands about what the row links to (their `ui.about`), as the
    // daemon says they apply to it now: the menu opens once it has.
    const ref = rowRef(viz, result.columns, row);
    if (!ref) return base;
    return refOffersFor(ref).then((entries) => [...base, ...refCommandMenuItems(entries)]);
  };
  const rowBase = (row: SqlCell[]): MenuItem[] => [
    {
      id: "ask-about-row",
      label: "Ask the Assistant",
      enabled: true,
      run: () => void askAssistant(rowAssistantContext(viz, result.columns, row)),
    },
    {
      id: "add-row-to-context",
      label: "Add to Context",
      enabled: true,
      run: () => void addToContext(rowContext(viz, result.columns, row)),
    },
    ...rowActions
      .filter((a) => actionOffered(a, rowRecord(result.columns, row)))
      .map((a) => ({
      id: `viz-action-${a.id}`,
      label: a.label,
      enabled: actions !== undefined,
      run: () => actions?.run(a, rowRecord(result.columns, row)),
    })),
  ];
  // Every row of every row component: focusable, with its menu from a
  // right-click or the keyboard (Menu key / Shift+F10).
  // A row that picks: a click (not on a link or control in it) or Enter
  // sets the param its column selects.
  // The row action a click or Enter runs: the viz's `primary`, where the row is offered it.
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
  const groups = rowGroups(viz, result);
  // A tree draws its rows in nesting order (a lazy tree's spliced rows too).
  const treeNodesDrawn = viz.viz === "tree" ? (tree?.nodes ?? treeNodes(viz, result)) : null;
  drawn = groups ? groups.filter((g) => !folded.has(g.key)).flatMap((g) => g.rows) : treeNodesDrawn ? drawnTreeRows(treeNodesDrawn) : result.rows;
  const drawnAt = new Map(drawn.map((r, i) => [r, i] as const));
  // Marks, drags and keys (`rowInteract.ts`).
  const drawnKeys = drawn.map(rowKey);
  const marked = marks ? shownMarks(marks.keys, drawnKeys) : [];
  const markedSet = new Set(marked);
  const recordOf = (row: SqlCell[]) => rowRecord(result.columns, row);
  const dragActions = viz.actions.filter((a) => a.drag);
  const onto = takesOnto(dragActions);
  const byIndex = viz.group ? result.columns.indexOf(viz.group.by) : -1;
  const groupOf = (row: SqlCell[]): SqlCell => (byIndex === -1 ? null : (row[byIndex] ?? null));
  const sameGroup = (a: SqlCell[], b: SqlCell[]) => byIndex === -1 || groupOf(a) === groupOf(b);
  // A marked row drags every marked row with it.
  const draggedWith = (row: SqlCell[]) => (markedSet.has(rowKey(row)) ? drawn.filter((r) => markedSet.has(rowKey(r))) : [row]);
  const carriesRows = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes(VIZ_ROW_DRAG_MIME);
  const readDrag = (e: React.DragEvent): SqlCell[][] | null => {
    try {
      const p = JSON.parse(e.dataTransfer.getData(VIZ_ROW_DRAG_MIME)) as { token?: unknown; keys?: unknown };
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
    const ref = rowContextRef(viz, result.columns, row);
    const emphasized = rowEmphasized(viz, result.columns, row);
    const drags = dragActions.some((a) => actionOffered(a, recordOf(row)));
    return {
      props: {
        ...(ref || drags
          ? {
              draggable: true,
              onDragStart: (e: React.DragEvent) => {
                if (ref) setContextRefDrag(e, ref);
                if (drags) {
                  e.dataTransfer.setData(VIZ_ROW_DRAG_MIME, JSON.stringify({ token: dragToken, keys: draggedWith(row).map(rowKey) }));
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
      indent: rowDepth(viz, result.columns, row) * DEPTH_INDENT,
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
      actions={viz.actions.filter((a) => a.group === g.key)}
      busy={actions?.busy ?? null}
      onAction={(a) => actions?.run(a, null)}
      onOpenPage={onOpenPage}
    />
    </div>
  );
  // Under a row: the viz each of its open `expand` columns names, run
  // with the row's values.
  const detail = (row: SqlCell[], index: number): ReactNode => {
    const open = cols.filter((c) => c.expand && expanded.has(expansionKey(c, row)));
    if (open.length === 0) return null;
    return (
      <div data-testid={`viz-row-expansion-${index}`} style={expansionStyle}>
        {open.map((c) => (
          <RowExpansion key={c.key} target={c.expand!} params={rowRecord(result.columns, row)} streamId={streamId} onOpenPage={onOpenPage} />
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
          const keyed = e.metaKey || e.ctrlKey || e.altKey ? null : keyAction(viz.actions, e.key);
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
      <Viz {...{ run, result, viz, cols, cell, first, compact, streamId, onOpenPage, rowMenu, rowsView, onCustomFailure, onCustomReady, componentView, fitCustom, onSetParam, isPage }} tree={treeNodesDrawn ? { nodes: treeNodesDrawn, open: tree?.open ?? null } : null} />
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

/** The param a viz's rows set when clicked (its column's `select`), if
 *  it has one and there's somewhere to set it. */
function rowPicks(run: VizRun, onSetParam: VizViewProps["onSetParam"]) {
  const column = run.config.columns.find((c) => c.select);
  const index = column ? run.result.columns.indexOf(column.key) : -1;
  if (!onSetParam || !column?.select || index === -1) return null;
  const name = column.select;
  const current = run.params[name] ?? null;
  const cols = displayColumns(run.config, run.result.columns);
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
  /** What a row opens under itself (its `expand` viz nodes), if anything. */
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
 *  (an arrow after it opens its page when the group's `link` resolves,
 *  never the text itself) and its group actions. */
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
  actions: DeclaredPageAction[];
  busy: string | null;
  onAction(a: DeclaredPageAction): void;
  onOpenPage?(ref: TabRef): void;
}) {
  const ref = group.ref;
  return (
    <div data-testid="viz-group-heading" data-group={group.key} style={groupHeadingStyle}>
      <button
        type="button"
        aria-expanded={open}
        aria-label={`${open ? "Collapse" : "Expand"} ${group.label}`}
        onClick={onToggle}
        style={disclosureStyle}
      >
        {open ? "▾" : "▸"}
      </button>
      <span>{group.label}</span>
      {ref ? <OpenPageLink to={ref} name={group.label} onNavigate={tabNavigate(ref, onOpenPage)} /> : null}
      <span data-testid="viz-group-count" style={{ fontWeight: "normal", color: "var(--text-muted)" }}>
        {group.rows.length}
      </span>
      {actions.length > 0 ? (
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 4 }}>
          {actions.map((a) => (
            <button
              key={a.id}
              type="button"
              data-testid={`viz-group-action-${a.id}`}
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

function Viz({
  run,
  result,
  viz,
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
  fitCustom,
  tree,
  onSetParam,
  isPage,
}: {
  run: VizRun;
  result: VizRun["result"];
  viz: VizRun["config"];
  /** Set one of the page's params (a terminal picked). */
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
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
  fitCustom: boolean;
  isPage: boolean;
}) {
  // A page file's `custom` viz is fed its page's loaders.
  const fed = useVizPages()?.componentData ?? null;
  switch (viz.viz) {
    case "bar":
      return (
        <div data-testid="viz-bar">
          <DailyBarChart rows={barRows(viz, result)} formatValue={(v) => formatMetricValue(v)} />
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
          problem={lazyTreeProblem(viz, result)}
          label={treeLabelColumn(viz, cols, result.columns)}
          cell={cell}
          rowMenu={rowMenu}
          rowLook={rowsView.rowLook}
        />
      );
    case "timeline":
      return <TimelineViz entries={timelineEntries(viz, result)} onOpenPage={onOpenPage} rowMenu={rowMenu} />;
    case "detail":
      return <DetailViz row={result.rows[0]!} cols={cols} cell={cell} rowMenu={rowMenu} />;
    case "sections":
      return (
        <SectionsViz
          node={viz.slug}
          sections={sectionRows(viz, result)}
          facts={(row) => cols.map((c) => ({ key: c.key, label: c.label, value: cell(row, c) }))}
          rowMenu={rowMenu}
          navigate={(link) => tabNavigate(link, onOpenPage)}
        />
      );
    case "steps":
      return <StepsViz steps={stepItems(viz, result)} rowMenu={rowMenu} />;
    case "approvals":
      // The pending proposals are read by the kit component itself; the
      // viz's rows are ignored, its params narrow the list.
      return <Approvals thread={paramText(run.params.thread)} setting={paramText(run.params.setting)} />;
    case "hunks":
      return <HunksViz rows={hunkRows(viz, result)} streamId={streamId} rowMenu={rowMenu} />;
    case "code":
      return <CodeViz rows={codeRows(viz, result)} streamId={streamId} sync={viz.code?.sync ?? false} onOpenPage={onOpenPage} isPage={isPage} />;
    case "diff":
      return <DiffViz spec={diffRow(viz, result)} streamId={streamId} onOpenPage={onOpenPage} />;
    case "editor":
      return <EditorViz row={editorRow(viz, result)} onOpenPage={onOpenPage} />;
    case "terminal":
      return <TerminalViz run={{ ...run, result }} onSetParam={onSetParam} />;
    case "embed":
      return <EmbedViz url={embedRow(viz, result)?.url ?? null} />;
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
          fit={fitCustom}
          data={fed}
        />
      );
    case "table":
    default:
      return <TableViz rows={result.rows} cols={cols} cell={cell} rowMenu={rowMenu} view={rowsView} />;
  }
}

/** `form`: the fields of its command's input, starting from the form's
 *  values; submitting runs the command as the viz, for the person, then
 *  goes where the command says (`ui.open_after`) as the form's `then`
 *  says. */
function FormViz({ run, streamId }: { run: VizRun; streamId: string | null }) {
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
  const [draft, setDraft, clearDraft] = useDraft<Drafts | null>(`form:${run.config.id}`, null);
  const openAfter = useOpenAfter();
  const source = useFormSource();
  const paramsKey = JSON.stringify(run.params);
  // Its loader's rows: a page run re-read after a save (or a change
  // elsewhere) starts it from what is saved now.
  const rowsKey = JSON.stringify(run.result.rows);
  const again = run.config.form?.again ?? null;
  const form = useFormSubmit(
    run.config.title,
    (input, confirmed) => source.submit(run, input, streamId, confirmed),
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
        void openAfterAction({ label: run.config.title, command: start.command.id, then: run.config.form?.then ?? "open" }, outcome, openAfter);
      }
    },
  );
  useEffect(() => {
    let live = true;
    source
      .start(run, streamId)
      .then((s) => {
        if (live) setStart(s);
      })
      .catch((e) => {
        if (live) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      live = false;
    };
    // paramsKey and rowsKey stand in for `run.params` and its rows.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run.config.id, paramsKey, rowsKey, streamId, round, reload, source]);
  useRerunOnChange(start?.reads ?? NO_READS, () => setReload((n) => n + 1));
  const schema = useMemo(
    () => (start ? withSubfields(start.command.input_schema as Record<string, unknown>, start.subfields ?? {}) : null),
    [start],
  );
  if (error) return <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)" }}>{error}</div>;
  if (!start || !schema) return null;
  const mode = run.config.form?.mode ?? "create";
  return (
    <div data-testid="viz-form">
      {form.pending ? (
        <CommandConfirm
          label={run.config.title}
          command={start.command.id}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
          testIdPrefix="viz-form-confirm"
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
        schemaHelp={false}
        mode={mode}
        submitLabel={mode === "edit" ? undefined : run.config.title}
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
        testIdPrefix="viz-form-fields"
      />
    </div>
  );
}

/** A form action's form (`form: true`): its command's fields starting
 *  from the action's bound input, its fixed fields shown but not editable;
 *  submitting runs it as the viz, for the person, and closes it. A form
 *  that is one text field is a prompt strip at the top of the viz
 *  (`promptField`); any other is a slideover. */
export function ActionForm({
  run,
  streamId,
  pressed,
  subject,
  onClose,
}: {
  run: ActionTarget;
  streamId: string | null;
  pressed: { action: DeclaredPageAction; row: Record<string, SqlCell> | null };
  /** The page's own title, which is what a page action is about: the
   *  subtitle of a form that has `fixed` fields (the host's ids, never
   *  drawn). A row action has none — its row isn't the page. */
  subject?: string | null;
  onClose(): void;
}) {
  const { action, row } = pressed;
  const [start, setStart] = useState<FormStart | null>(null);
  const openAfter = useOpenAfter();
  const pressAction = useVizActionRunner();
  const source = useFormSource();
  const form = useFormSubmit(
    action.label,
    (input, confirmed) => pressAction(run.config.id, action.id, run.params, row, streamId, confirmed, input),
    (outcome) => {
      onClose();
      void openAfterAction(action, outcome, openAfter);
    },
  );
  const rowKey = JSON.stringify(row);
  useEffect(() => {
    let live = true;
    source
      .actionStart(run, action.id, row, streamId)
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
  }, [run.config.id, action.id, rowKey, streamId, source]);
  // Which shape opens is the form's to say: nothing until it is read.
  if (!start) return null;
  // A menu label's ellipsis says "this asks first"; the form that asks is
  // headed, and its button labelled, without it.
  const name = actionName(action.label);
  // What the form is about, in words: the row's `about` column, else — for
  // a page's own action with fixed fields — the page's title. Never an id.
  const aboutCell = action.about && row ? row[action.about] : null;
  const about =
    aboutCell !== null && aboutCell !== undefined && String(aboutCell) !== ""
      ? String(aboutCell)
      : row === null && start.fixed.length > 0
        ? (subject ?? null)
        : null;
  const field = promptField(start);
  if (field) {
    return <ActionPrompt label={name} about={about} start={start} field={field} form={form} submitLabel={name} onClose={onClose} testId="viz-action-form" />;
  }
  return <FormModal title={name} subtitle={about} start={start} form={form} submitLabel={name} onClose={onClose} testId="viz-action-form" />;
}

/** An action's label as a form's heading and button: the menu label without
 *  its trailing ellipsis (`Rename…`, `Rename...`). */
export function actionName(label: string): string {
  return label.replace(/\s*(…|\.{3})$/, "");
}

/** A form that is one text field, as a prompt strip at the top of the
 *  viz (the Project panel's Rename… is the same strip): the field's
 *  description is its message, the bound value is selected to type over,
 *  Enter runs, Escape cancels. A secret field is masked, and blank leaves
 *  it out (it keeps what is saved). A command that asks is confirmed above it;
 *  a refusal shows under it and the strip stays with what was typed.
 *  `testId` is the slideover's, so whoever presses the action finds the
 *  field and the submit at the same ids whichever shape opens. */
function ActionPrompt({
  label,
  about,
  start,
  field,
  form,
  submitLabel,
  onClose,
  testId,
}: {
  label: string;
  /** What it is about, in words: ahead of the field's own message. */
  about: string | null;
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
  const fieldSpec = start.fields ? shownFields(start.fields).find((f) => f.name === field.key) : undefined;
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
        message={[about, fieldSpec?.help ?? fieldSpec?.label ?? field.label].filter(Boolean).join(" — ")}
        fields={[{ key: field.key, initialValue: String(values[field.key] ?? ""), placeholder: fieldSpec?.label ?? field.label, optional, secret: field.secret }]}
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
export function FormModal({
  title,
  subtitle,
  start,
  form,
  submitLabel,
  onClose,
  testId,
}: {
  title: string;
  /** What the form is about, under its title. */
  subtitle?: string | null;
  start: FormStart | null;
  form: ReturnType<typeof useFormSubmit>;
  submitLabel: string;
  onClose(): void;
  testId: string;
}) {
  return (
    <TabModal onClose={onClose} title={title} subtitle={subtitle} testId={testId}>
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
          schemaHelp={false}
          submitLabel={submitLabel}
          busy={form.busy}
          onSubmit={(input) => void form.submit(input, false)}
          onCancel={onClose}
          testIdPrefix={`${testId}-fields`}
        />
      ) : null}
    </TabModal>
  );
}

/** The column a tree's labels are drawn as: its declared column (so its
 *  link, icon and tone apply), else the plain `tree.label` column. */
function treeLabelColumn(viz: VizRun["config"], cols: DisplayColumn[], resultColumns: string[]): DisplayColumn | null {
  const key = viz.tree?.label;
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
  const closed = useCollapsedSet("viz-tree.closed");
  return (
    <>
      {problem ? (
        <div className="ox-error" data-testid="viz-tree-problem">
          {problem}
        </div>
      ) : null}
      <ul data-testid="viz-tree" style={treeListStyle}>
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
    <li data-testid="viz-tree-node">
      <div
        data-testid="viz-tree-row"
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
        <div data-testid="viz-tree-loading" style={{ ...treeNoteStyle }}>
          Loading…
        </div>
      ) : null}
      {open && node.state && node.state !== "loading" ? (
        <div className="ox-error" data-testid="viz-tree-error" style={treeNoteStyle}>
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

/** What an `expand` column's row opens: the page it names, drawn by the
 *  page host (`VizPagesContext`); outside a page node, nothing. */
function RowExpansion(props: {
  target: string;
  params: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
}) {
  const pages = useVizPages();
  return pages ? <pages.Expansion target={props.target} params={props.params} streamId={props.streamId} onOpenPage={props.onOpenPage} /> : null;
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
    <ol data-testid="viz-timeline" style={{ listStyle: "none", padding: 0, margin: 0, borderLeft: "2px solid var(--border-subtle)" }}>
      {entries.map((e, i) => {
        const ref = e.ref && parseUri(e.ref) ? e.ref : null;
        return (
          <li key={i} data-testid={`viz-timeline-entry-${i}`} style={{ padding: "4px 0 4px 12px" }} {...rowMenu(e.row)}>
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

/** `detail`: the first row as label/value pairs. An empty value reads
 *  "(none)", a word rather than a mark beside its label. */
function DetailViz({ row, cols, cell, rowMenu }: { row: SqlCell[]; cols: DisplayColumn[]; cell: CellRenderer; rowMenu: RowMenu }) {
  const value = (c: DisplayColumn) => ((row[c.index] ?? null) === null ? <span className="ox-faint">(none)</span> : cell(row, c));
  return <Facts testId="viz-detail" facts={cols.map((c) => ({ key: c.key, label: c.label, value: value(c) }))} {...rowMenu(row)} />;
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
    <ol data-testid="viz-steps" style={{ listStyle: "none", padding: 0, margin: 0 }}>
      {steps.map((s, i) => {
        const m = STEP_MARK[s.status];
        return (
          <li key={i} data-testid={`viz-step-${i}`} data-status={s.status} style={{ display: "flex", gap: 8, padding: "3px 0" }} {...rowMenu(s.row)}>
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
  const openSet = useCollapsedSet("viz-hunks.open", rows.length === 1 ? ["0"] : []);
  const open = openSet.ids.length > 0 ? Number(openSet.ids[0]) : null;
  const setOpen = (next: (o: number | null) => number | null) => {
    const to = next(open);
    openSet.set(to === null ? [] : [String(to)]);
  };
  return (
    <div data-testid="viz-hunks">
      {rows.map((r, i) => (
        <section key={i} data-testid={`viz-hunk-${i}`} style={{ borderBottom: "1px solid var(--border-subtle)" }} {...rowMenu(r.row)}>
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


function LineViz({ run }: { run: VizRun }) {
  const series = lineSeries(run.config, run.result);
  return (
    <div data-testid="viz-line">
      {series.map((s) => (
        <div key={s.name} data-testid="viz-line-series" style={{ marginBottom: 12 }}>
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
 *  items inside each group. A tile follows the viz's first column link. */
function TreemapViz({ run, onOpenPage }: { run: VizRun; onOpenPage?(ref: TabRef): void }) {
  const nav = useOptionalPageNavigation();
  const items = treemapItems(run.config, run.result);
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
  const linkCol = run.config.columns.find((c) => c.link);
  const open = (row: SqlCell[]) => {
    if (!linkCol?.link) return;
    const ref = cellLinkRef(linkCol.link, linkCol.key, row, run.result.columns);
    if (typeof ref === "string") (nav?.openRef ?? openRefInWindow)(ref, {});
    else if (ref) onOpenPage?.(ref);
  };
  return (
    <div data-testid="viz-treemap">
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
              data-testid="viz-treemap-tile"
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
      <span data-testid="viz-number" title={title} style={{ color: "var(--text-secondary)", fontWeight: 500 }}>
        {text}
      </span>
    );
  }
  return <Stat testId="viz-number" big value={text} title={title} />;
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
 *  test id), under their group's heading when the viz is grouped. */
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
      <ul data-testid="viz-list" style={{ listStyle: "none", padding: 0, margin: 0 }} {...(view?.cursor.listProps as React.HTMLAttributes<HTMLUListElement>)}>
        {sections(rows, view).map(({ group, rows: inGroup }) => (
          <Fragment key={group?.key ?? ""}>
            {group && view ? <li style={{ listStyle: "none" }}>{view.heading(group)}</li> : null}
            {inGroup.map(([row, i]) => {
              const look = view?.rowLook(row) ?? noLook;
              return (
                <li
                  key={i}
                  data-testid={`viz-row-${i}`}
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
                        data-testid={`viz-row-fold-${i}`}
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
    <div data-testid="viz-board" className="ox-board" {...(view?.cursor.listProps as React.HTMLAttributes<HTMLDivElement>)}>
      {(view?.groups ?? []).map((g) => (
        <section key={g.key} data-testid={`viz-board-column-${g.key}`} className="ox-board-column" {...view?.groupDrop(g)}>
          {view?.heading(g)}
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {(view?.folded(g) ? [] : g.rows).map((row) => {
              const i = index.get(row) ?? 0;
              const look = view?.rowLook(row) ?? noLook;
              return (
                <li
                  key={i}
                  data-testid={`viz-row-${i}`}
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
    wrap: columnWrap(
      c.format,
      c.unitIndex !== null,
      rows.map((row) => formatCell(row[c.index] ?? null, c.unitIndex === null ? undefined : (row[c.unitIndex] ?? null), c.format)),
    ),
    render: ([row]) => cell(row, c),
  }));
  const parts = sections(rows, view);
  return (
    <DataTable<Row>
      testId="viz-table"
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
          "data-testid": `viz-row-${i}`,
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

/** A viz param as text, or null when it is unset or empty. */
function paramText(value: SqlCell | undefined): string | null {
  return value === undefined || value === null || value === "" ? null : String(value);
}
