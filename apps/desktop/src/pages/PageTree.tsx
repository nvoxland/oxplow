/// A page file's run, drawn (`.context/pages-and-tabs.md` "Page files"):
/// the server resolved the tree (a switch to its child in view, an embed
/// to its page's run), so this draws what `RunNode`s it was given, each
/// looked up by id in the run's page. A viz node is drawn by the page viz
/// components (`VizView`) as the `VizRun` `nodeRunOf` makes of it,
/// its actions pressed on the open run (`useNodeActions`).
///
/// Every node's root carries `data-node` with its id (an embed's nodes
/// `<embed>/<id>`, as the server names them for actions) and registers in
/// the `NodeRegistry`, so the host can land on `#node=<id>`.
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";

import { Markdown } from "../components/Markdown/Markdown.js";
import { Replaced } from "../components/Replaced.js";
import { Section } from "../kit/Section.js";
import { Tile, TileGrid, TileSlot } from "../kit/Tiles.js";
import type { PickedRow } from "./railGroups.js";
import { FormSourceContext } from "../viz/formSource.js";
import { VizActionRunnerContext } from "../viz/vizActionRunner.js";
import { ActionForm, VizView, useVizActions } from "../viz/VizView.js";
import { actionDoes, offeredWholePage } from "../viz/vizActions.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { nodeById, nodeRunOf } from "../viz/nodeRun.js";
import { foldEmpty } from "../viz/vizModel.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Node, Page, PageRun, RunNode, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageActionContext, PageFormsContext, useNodeActions, useNodeForms } from "./useNodeActions.js";
import { usePageBranches } from "./pageBranches.js";
import { ComponentNode } from "./PageComponent.js";
import { pageData } from "../viz/componentBridge.js";
import { declaredParams, namedLoader, parsePageRef, usePageReads } from "./pageReads.js";
import { firstLoader } from "./firstLoader.js";
import { VizPagesContext, type ExpansionProps, type VizPages } from "../viz/vizPages.js";
import { HostedViewContext } from "../viz/hostedView.js";
import { usePageRunsStore, usePageRunWith } from "../pageRuns.js";
import { DashboardGrid } from "./DashboardGrid.js";

// --- The registry --------------------------------------------------------

/** The drawn nodes' root elements, by (prefixed) id. */
export interface NodeRegistry {
  register(id: string, el: HTMLElement | null): void;
  get(id: string): HTMLElement | undefined;
  /** Told each time a node registers; returns an unsubscribe. */
  subscribe(listener: () => void): () => void;
}

export function createNodeRegistry(): NodeRegistry {
  const els = new Map<string, HTMLElement>();
  const listeners = new Set<() => void>();
  return {
    register(id, el) {
      if (!el) {
        els.delete(id);
        return;
      }
      els.set(id, el);
      for (const l of [...listeners]) l();
    },
    get: (id) => els.get(id),
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

export const NodeRegistryContext = createContext<NodeRegistry | null>(null);

/** How long a node landed on stays marked. */
export const FOCUS_MS = 1500;

/** Land on node `focusId` (a link's `#node=<id>`): scroll it into view and
 *  mark it (`data-focused`) for `ms`, once per id — when it is drawn, or as
 *  soon as it registers. */
export function useNodeFocus(registry: NodeRegistry, focusId: string | null, ms: number = FOCUS_MS): void {
  useEffect(() => {
    if (!focusId) return;
    let landed: HTMLElement | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const unmark = () => {
      if (!landed) return;
      landed.removeAttribute("data-focused");
      landed.style.outline = "";
    };
    const land = () => {
      if (landed) return;
      const el = registry.get(focusId);
      if (!el) return;
      landed = el;
      el.scrollIntoView?.({ block: "start" });
      el.setAttribute("data-focused", "true");
      // The kit stylesheet has no rule for it yet.
      el.style.outline = "2px solid var(--accent)";
      timer = setTimeout(unmark, ms);
    };
    land();
    const off = registry.subscribe(land);
    return () => {
      off();
      if (timer) clearTimeout(timer);
      unmark();
    };
  }, [registry, focusId, ms]);
}

// --- The tree ------------------------------------------------------------

export interface PageTreeProps {
  run: PageRun;
  /** The open run its actions are pressed on; null until it opens. */
  runId: string | null;
  /** What to draw: the run's view (default) or its rail. */
  root?: RunNode;
  /** An embedded page's nodes' prefix (`<embed>/`). */
  prefix?: string;
  streamId?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  /** Set one of the page's params (a row that picks); without it rows
   *  are no picks. */
  onSetParam?(name: string, value: SqlCell, row?: PickedRow): void;
  /** Drawn small (a rail panel): its vizes compact, a few rows each. */
  compact?: boolean;
  /** Its vizes' whole-viz actions as buttons above them; default: unless
   *  compact (an expanded row's page is compact and keeps them). */
  partButtons?: boolean;
}

/** How a node sits in its parent: on its own, a stack's section, a
 *  grid's tile. */
type Placement = "bare" | "section" | "tile";

interface NodeViewProps extends Omit<PageTreeProps, "root"> {
  at: RunNode;
  placement: Placement;
}

export function PageTree(props: PageTreeProps) {
  const { root = props.run.tree, ...rest } = props;
  return <NodeView {...rest} at={root} placement="bare" />;
}

function NodeView(props: NodeViewProps) {
  const { run, at, prefix = "", placement } = props;
  const def = nodeById(run.page, at.id);
  const id = `${prefix}${at.id}`;
  const own = at.error ? <NodeError message={at.error} /> : def ? <NodeBody {...props} def={def} id={id} /> : <Unknown {...props} />;
  // A node another extension's page stands in for (`replaceable:
  // work_item.board`): the run decided whose and ran it, drawn as an embed.
  const body =
    at.replacedBy && def?.replaceable ? (
      <Replaced
        target={def.replaceable}
        extension={at.replacedBy}
        error={at.error}
        fallback={<NodeBody {...props} def={def} id={id} />}
        streamId={props.streamId ?? null}
      >
        {at.embed ? <PageTree {...props} run={at.embed} root={at.embed.tree} prefix={`${id}/`} onSetParam={undefined} /> : null}
      </Replaced>
    ) : (
      own
    );
  // A page a slot mounted, or an embed with no title, is titled by its own
  // page: an embed's heading is not drawn.
  const title = def?.title || (def?.kind === "embed" || !def ? (at.embed?.page.title ?? null) : null);
  const hideTitle = def ? def.hideTitle || (def.kind === "embed" && !def.title) : false;
  const fills = fillsNode(run, at);
  return (
    <NodeFrame id={id} fill={fills}>
      {placement === "section" && title ? (
        <Section id={id} title={title} help={def?.description ?? undefined} hideTitle={hideTitle} fill={fills}>
          {body}
        </Section>
      ) : placement === "tile" ? (
        <Tile title={title ?? undefined} hideTitle={hideTitle}>
          {body}
        </Tile>
      ) : (
        body
      )}
    </NodeFrame>
  );
}

/** Keep the height chain intact through stacks and the selected switch
 * branch: terminals only start once their surface has a visible size. */
function fillsNode(run: PageRun, at: RunNode): boolean {
  const def = nodeById(run.page, at.id);
  if (def?.kind === "viz") return def.config.fills || (FILLS_ALONE.has(def.config.viz) && isWholePage(run, at.id));
  if (def?.kind === "stack" && at.children.length === 1) {
    // A stack of one is its child's height.
    return fillsNode(run, at.children[0]!);
  }
  return (def?.kind === "stack" || def?.kind === "switch") && at.children.some((child) => fillsNode(run, child));
}

/** The vizzes that take the page's height when they are all of it: a
 *  history list should not sit five rows tall in an empty page. */
const FILLS_ALONE: ReadonlySet<string> = new Set(["table", "list", "tree", "custom"]);

/** Whether node `id` is the whole of its page's body: its view, or the only
 *  node of a view that is a stack. */
function isWholePage(run: PageRun, id: string): boolean {
  const view = run.page.view;
  if (view.id === id) return true;
  return view.kind === "stack" && view.children.length === 1 && view.children[0]!.id === id;
}

/** A node's root: where `#node=` lands. */
function NodeFrame({ id, fill, children }: { id: string; fill: boolean; children: ReactNode }) {
  const registry = useContext(NodeRegistryContext);
  const ref = useCallback((el: HTMLDivElement | null) => registry?.register(id, el), [registry, id]);
  return (
    <div data-node={id} data-testid={`node-${id}`} ref={ref} style={fill ? fillStyle : undefined}>
      {children}
    </div>
  );
}

function NodeBody(props: NodeViewProps & { def: Node; id: string }) {
  const { def, at, id } = props;
  const kids = (placement: Placement) => at.children.map((c) => <NodeView key={c.id} {...props} at={c} placement={placement} />);
  switch (def.kind) {
    case "stack": {
      // Titled sections with nothing to show fold into one line, as a page
      // document's parts do (`foldEmpty`).
      const folded = emptySections(props.run, at);
      const shown = at.children.filter((c) => !folded.some((f) => f.id === c.id));
      return (
        <>
          {shown.map((c) => (
            <NodeView key={c.id} {...props} at={c} placement="section" />
          ))}
          {folded.length > 0 ? (
            <p data-testid="page-nothing-found" className="ox-empty-line">
              Nothing found: {folded.map((f) => f.title).join(", ")}.
            </p>
          ) : null}
        </>
      );
    }
    case "grid":
      if (props.run.page.category === "Dashboards" && props.run.page.view.id === def.id && !props.run.page.origin.bundled) {
        return <DashboardGrid page={props.run.page.id} tiles={at.children.map((c) => ({
          id: c.id, size: nodeById(props.run.page, c.id)?.size ?? "small", body: <NodeView {...props} at={c} placement="tile" />,
        }))} />;
      }
      return (
        <TileGrid>
          {at.children.map((c) => (
            <TileSlot key={c.id} size={nodeById(props.run.page, c.id)?.size ?? "small"}>
              <NodeView {...props} at={c} placement="tile" />
            </TileSlot>
          ))}
        </TileGrid>
      );
    case "switch":
      // The server resolved it: its one child in view.
      return <>{kids("bare")}</>;
    case "slot":
      // What other extensions mounted here; none is nothing. A mounted page
      // with nothing to show is left out, not drawn as a heading over "Nothing found".
      return <>{at.children.filter((c) => !mountEmpty(c)).map((c) => <NodeView key={c.id} {...props} at={c} placement="section" />)}</>;
    case "markdown":
      return <Markdown value={def.text} />;
    case "component":
      // The page's extension's component, fed the page's loaders.
      return <ComponentNode run={props.run} def={def} streamId={props.streamId ?? null} />;
    case "embed":
      return at.embed ? (
        <>
          {props.compact ? null : <EmbeddedActions embed={at.embed} node={id} runId={props.runId} streamId={props.streamId ?? null} />}
          <PageTree {...props} run={at.embed} root={at.embed.tree} prefix={`${id}/`} onSetParam={undefined} />
        </>
      ) : null;
    case "viz":
      return <VizNode {...props} def={def} />;
  }
}

/** An embedded page's own (whole-page) actions, as buttons above it:
 *  pressed on the host's run naming the embed's node, which the server
 *  runs as that page's action. */
function EmbeddedActions({ embed, node, runId, streamId }: { embed: PageRun; node: string; runId: string | null; streamId: string | null }) {
  const runner = useNodeActions(runId, node);
  const forms = useNodeForms(runId, node);
  const target = useMemo(() => ({ config: { id: embed.page.id }, params: embed.params }), [embed.page.id, embed.params]);
  const actions = useVizActions(target, streamId, runner);
  const first = firstLoader(embed.page)?.name;
  const firstResult = first ? embed.loaders[first]?.result : undefined;
  const offered = embed.page.actions.filter((a) => offeredWholePage(a, firstResult ?? { columns: [], rows: [] }));
  if (offered.length === 0 && !actions.pending && !actions.form) return null;
  return (
    <>
      {actions.pending ? (
        <CommandConfirm
          label={actions.pending.action.label}
          command={actions.pending.action.command ?? ""}
          details={actions.pending.details}
          onConfirm={actions.confirm}
          onCancel={actions.cancel}
          testIdPrefix="viz-action-confirm"
        />
      ) : null}
      {actions.form ? (
        <VizActionRunnerContext.Provider value={runner}>
          <FormSourceContext.Provider value={forms}>
            <ActionForm run={target} streamId={streamId} pressed={actions.form} onClose={actions.closeForm} />
          </FormSourceContext.Provider>
        </VizActionRunnerContext.Provider>
      ) : null}
      {offered.length > 0 ? (
        <div data-testid="viz-part-actions" className="ox-toolbar">
          {offered.map((a) => (
            <button key={a.id} type="button" data-testid={`viz-action-${a.id}`} disabled={actions.busy !== null} title={actionDoes(a)} onClick={() => actions.run(a, null)}>
              {actions.busy === a.id ? `${a.label}…` : a.label}
            </button>
          ))}
        </div>
      ) : null}
    </>
  );
}

/** A child the page doesn't declare: what a slot mounted, an embed run. */
function Unknown(props: NodeViewProps) {
  const { at, prefix = "" } = props;
  if (!at.embed) return null;
  return <PageTree {...props} run={at.embed} root={at.embed.tree} prefix={`${prefix}${at.id}/`} onSetParam={undefined} />;
}

function VizNode(props: NodeViewProps & { def: Node; id: string }) {
  const { run, def, id, runId, streamId = null, onOpenPage, onSetParam, compact = false } = props;
  // The viz is the page's whole view: the root of its own tree, drawn at full
  // size (not a rail, an embed's page or an expanded row's).
  const isPage = props.at === run.tree && props.prefix === undefined && !compact && props.placement === "bare";
  const runner = useNodeActions(runId, id);
  const forms = useNodeForms(runId, id);
  const pages = useNodePages(run, def);
  const componentView = useContext(HostedViewContext);
  const vizRun = nodeRunOf(run, def);
  if (!vizRun || def.kind !== "viz") return null;
  const failed = def.from !== null ? run.loaders[def.from]?.error : null;
  if (failed) return <NodeError message={failed} />;
  const lead = leadOf(vizRun);
  return (
    <VizActionRunnerContext.Provider value={runner}>
    <FormSourceContext.Provider value={forms}>
    <VizPagesContext.Provider value={pages}>
      {lead ? (
        <p className="ox-section-desc" data-testid={`node-${id}-lead`}>
          {lead}
        </p>
      ) : null}
      <VizView
        run={vizRun}
        streamId={streamId}
        onOpenPage={onOpenPage}
        onSetParam={onSetParam}
        partButtons={props.partButtons ?? !compact}
        isPage={isPage}
        componentView={componentView}
        {...(compact ? { compact: true, maxRows: COMPACT_ROWS } : {})}
      />
    </VizPagesContext.Provider>
    </FormSourceContext.Provider>
    </VizActionRunnerContext.Provider>
  );
}

/** A node's `lead`: the first row's text of its column, a paragraph under
 *  the heading; none when the column is missing, null or empty. */
export function leadOf(run: { config: { lead?: string | null }; result: { columns: string[]; rows: unknown[][] } }): string | null {
  const column = run.config.lead;
  if (!column) return null;
  const at = run.result.columns.indexOf(column);
  const cell = at < 0 ? null : run.result.rows[0]?.[at];
  return typeof cell === "string" && cell.trim() !== "" ? cell.trim() : null;
}

/** The children of stack `at` that fold away: titled, not replaceable
 *  viz nodes whose loader ran and has nothing for them (`foldEmpty`). */
export function emptySections(run: PageRun, at: RunNode): { id: string; title: string }[] {
  const candidates = at.children.flatMap((c) => {
    const def = nodeById(run.page, c.id);
    if (!def || def.kind !== "viz" || !def.title || def.hideTitle || def.replaceable || c.error) return [];
    const vizRun = nodeRunOf(run, def);
    const loader = def.from !== null ? run.loaders[def.from] : undefined;
    if (!vizRun || !loader) return [];
    return [{ id: c.id, title: def.title, run: vizRun, error: loader.error }];
  });
  return foldEmpty(candidates).empty.map(({ id, title }) => ({ id, title }));
}

/** Whether a page another extension mounted (a slot's child) has nothing
 *  to show: its run is there, no error, and every viz in it has no rows
 *  (as `foldEmpty` counts it). A mount that failed or has not run is not empty. */
export function mountEmpty(at: RunNode): boolean {
  const run = at.embed;
  if (!run || at.error) return false;
  const walk = (n: RunNode): boolean => {
    const def = nodeById(run.page, n.id);
    if (!def || n.error) return false;
    if (def.kind === "viz") {
      const vizRun = nodeRunOf(run, def);
      const loader = def.from !== null ? run.loaders[def.from] : undefined;
      if (!vizRun || !loader) return false;
      return foldEmpty([{ run: vizRun, error: loader.error }]).empty.length === 1;
    }
    if (def.kind === "stack" || def.kind === "grid") return n.children.length > 0 && n.children.every(walk);
    return false;
  };
  return walk(run.tree);
}

/** What viz node `def` of `run` reads of pages: an `expand` column's
 *  page, its lazy branches' runs of its own page. */
function useNodePages(run: PageRun, def: Node): VizPages {
  const store = usePageRunsStore();
  const press = useContext(PageActionContext);
  const forms = useContext(PageFormsContext);
  return useMemo(() => {
    const host = { pageRun: run, node: def, store, press, forms };
    const custom = def.kind === "viz" && def.config.viz === "custom";
    return {
      Expansion: PageExpansion,
      useBranches: (vizRun, open, streamId, maxRows) => usePageBranches(host, vizRun, open, streamId, maxRows),
      // A `custom` viz is fed every loader of its page, its own rows its run.
      componentData: custom ? pageData(run, def.config.custom?.props ?? null, nodeRunOf(run, def)) : null,
    };
  }, [run, def, store, press, forms]);
}

/** What an `expand` column's row opens: the page it names (`<ext>/<page>`
 *  or `<ext>/<page>#<loader>`) as a run of its own, given the row's values
 *  it declares, its named loader's node drawn small (none draws it: the
 *  whole page). */
function PageExpansion({ target, params, streamId, onOpenPage }: ExpansionProps) {
  const { page, loader } = parsePageRef(target);
  const reads = usePageReads();
  const [def, setDef] = useState<{ id: string; page: Page | null; error: string | null } | null>(null);
  useEffect(() => {
    let live = true;
    reads.page(page).then(
      (p) => live && setDef({ id: page, page: p, error: null }),
      (e: unknown) => live && setDef({ id: page, page: null, error: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      live = false;
    };
  }, [page, reads]);
  const declared = def?.id === page && def.page ? def.page : null;
  const store = usePageRunsStore();
  const state = usePageRunWith(store, declared ? { page, params: declaredParams(declared, params), streamId, threadId: null } : null);
  const error = (def?.id === page ? def.error : null) ?? state.error;
  if (error) return <p className="ox-error">{error}</p>;
  if (!state.run) return <p className="ox-muted ox-small">Loading…</p>;
  const named = namedLoader(state.run, loader);
  const shows = named ? vizReading(state.run.page, named.name) : null;
  const root: RunNode = shows ? ({ id: shows.id, children: [], embed: null, error: null } as RunNode) : state.run.tree;
  return (
    // Its nodes are its own page's: nothing in the host lands on them.
    <NodeRegistryContext.Provider value={null}>
      <PageTree run={state.run} runId={state.runId} root={root} streamId={streamId} onOpenPage={onOpenPage} compact partButtons />
    </NodeRegistryContext.Provider>
  );
}

/** The first viz node of `page` that draws loader `name`. */
function vizReading(page: Page, name: string): Node | null {
  const walk = (n: Node): Node | null => {
    if (n.kind === "viz" && n.from === name) return n;
    if (n.kind === "stack" || n.kind === "grid" || n.kind === "switch") {
      for (const c of n.children) {
        const hit = walk(c);
        if (hit) return hit;
      }
    }
    return null;
  };
  return walk(page.view) ?? (page.rail ? walk(page.rail) : null);
}

function NodeError({ message }: { message: string }) {
  return (
    <div className="ox-error" role="alert">
      {message}
    </div>
  );
}

/** How many rows a compact viz shows. */
const COMPACT_ROWS = 8;

const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };
