/// A page file's run, drawn (`.context/pages-and-tabs.md` "Page files"):
/// the server resolved the tree (a switch to its child in view, an embed
/// to its page's run), so this draws what `RunNode`s it was given, each
/// looked up by id in the run's page. A viz node is drawn by the lens viz
/// components (`LensResultView`) as the `LensRun` `nodeRunOf` makes of it,
/// its actions pressed on the open run (`useNodeActions`).
///
/// Every node's root carries `data-node` with its id (an embed's nodes
/// `<embed>/<id>`, as the server names them for actions) and registers in
/// the `NodeRegistry`, so the host can land on `#node=<id>`.
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";

import { Markdown } from "../components/Markdown/Markdown.js";
import { Replaceable } from "../components/Replaceable.js";
import { Section } from "../kit/Section.js";
import { Tile, TileGrid, TileSlot } from "../kit/Tiles.js";
import type { PickedRow } from "../lens/LensRail.js";
import { FormSourceContext } from "../lens/formSource.js";
import { LensActionRunnerContext } from "../lens/lensActionRunner.js";
import { LensResultView } from "../lens/LensResultView.js";
import { nodeById, nodeRunOf } from "../lens/nodeRun.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Node, Page, PageRun, RunNode, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageActionContext, PageFormsContext, useNodeActions, useNodeForms } from "./useNodeActions.js";
import { usePageBranches } from "./pageBranches.js";
import { declaredParams, namedLoader, parsePageRef, usePageReads } from "./pageReads.js";
import { VizPagesContext, type ExpansionProps, type VizPages } from "../lens/vizPages.js";
import { usePageRunsStore, usePageRunWith } from "../pageRuns.js";

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
  // A node another extension's provider may take the place of
  // (`replaceable: work_item.board`): the same choice as a lens part's.
  const body = def?.replaceable ? (
    <Replaceable target={def.replaceable} props={valued(run.params)} streamId={props.streamId ?? null} onOpenPage={props.onOpenPage} fallback={own} />
  ) : (
    own
  );
  const title = def?.title ?? null;
  const fills = def?.kind === "viz" && def.lens.fills;
  return (
    <NodeFrame id={id} fill={fills}>
      {placement === "section" && title ? (
        <Section id={id} title={title} fill={fills}>
          {body}
        </Section>
      ) : placement === "tile" ? (
        <Tile title={title ?? undefined}>{body}</Tile>
      ) : (
        body
      )}
    </NodeFrame>
  );
}

/** A node's root: where `#node=` lands. */
function NodeFrame({ id, fill, children }: { id: string; fill: boolean; children: ReactNode }) {
  const registry = useContext(NodeRegistryContext);
  const ref = useCallback((el: HTMLDivElement | null) => registry?.register(id, el), [registry, id]);
  return (
    <div data-node={id} ref={ref} style={fill ? fillStyle : undefined}>
      {children}
    </div>
  );
}

function NodeBody(props: NodeViewProps & { def: Node; id: string }) {
  const { def, at, id } = props;
  const kids = (placement: Placement) => at.children.map((c) => <NodeView key={c.id} {...props} at={c} placement={placement} />);
  switch (def.kind) {
    case "stack":
      return <>{kids("section")}</>;
    case "grid":
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
      // What other extensions mounted here; none is nothing.
      return <>{kids("section")}</>;
    case "markdown":
      return <Markdown value={def.text} />;
    case "component":
      // The custom component bridge comes with a later step.
      return <div data-component={def.component} />;
    case "embed":
      return at.embed ? <PageTree {...props} run={at.embed} root={at.embed.tree} prefix={`${id}/`} onSetParam={undefined} /> : null;
    case "viz":
      return <VizNode {...props} def={def} />;
  }
}

/** A child the page doesn't declare: what a slot mounted, an embed run. */
function Unknown(props: NodeViewProps) {
  const { at, prefix = "" } = props;
  if (!at.embed) return null;
  return <PageTree {...props} run={at.embed} root={at.embed.tree} prefix={`${prefix}${at.id}/`} onSetParam={undefined} />;
}

function VizNode(props: NodeViewProps & { def: Node; id: string }) {
  const { run, def, id, runId, streamId = null, onOpenPage, onSetParam, compact = false } = props;
  const runner = useNodeActions(runId, id);
  const forms = useNodeForms(runId, id);
  const pages = useNodePages(run, def);
  const lensRun = nodeRunOf(run, def);
  if (!lensRun || def.kind !== "viz") return null;
  const failed = def.from !== null ? run.loaders[def.from]?.error : null;
  if (failed) return <NodeError message={failed} />;
  return (
    <LensActionRunnerContext.Provider value={runner}>
    <FormSourceContext.Provider value={forms}>
    <VizPagesContext.Provider value={pages}>
      <LensResultView
        run={lensRun}
        streamId={streamId}
        onOpenPage={onOpenPage}
        onSetParam={onSetParam}
        toolbar={false}
        partButtons={!compact}
        {...(compact ? { compact: true, maxRows: COMPACT_ROWS } : {})}
      />
    </VizPagesContext.Provider>
    </FormSourceContext.Provider>
    </LensActionRunnerContext.Provider>
  );
}

/** What viz node `def` of `run` reads of pages: an `expand` column's
 *  page, its lazy branches' runs of its own page. */
function useNodePages(run: PageRun, def: Node): VizPages {
  const store = usePageRunsStore();
  const press = useContext(PageActionContext);
  const forms = useContext(PageFormsContext);
  return useMemo(() => {
    const host = { pageRun: run, node: def, store, press, forms };
    return {
      Expansion: PageExpansion,
      useBranches: (lensRun, open, streamId, maxRows) => usePageBranches(host, lensRun, open, streamId, maxRows),
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
      <PageTree run={state.run} runId={state.runId} root={root} streamId={streamId} onOpenPage={onOpenPage} compact />
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

/** The params that hold a value: what a replacement is given, so one the
 *  page left empty (`head_sha` before it resolves) resolves from the
 *  viewer's context there too rather than binding NULL. */
function valued(params: Record<string, SqlCell>): Record<string, SqlCell> {
  return Object.fromEntries(Object.entries(params).filter(([, v]) => v !== null && v !== undefined));
}

/** How many rows a compact viz shows. */
const COMPACT_ROWS = 8;

const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };
