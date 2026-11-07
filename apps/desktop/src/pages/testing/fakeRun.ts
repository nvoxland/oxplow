/// Fake page runs for the page host's tests: a `PageRun` built from a few
/// nodes and loaders, and a `PageRuns` store over a transport that answers
/// with it (no daemon).
import { createPageRuns, type PageKey, type PageRunsTransport, type RunChange } from "../../pageRuns.js";
import type {
  VizConfig,
  PageAction,
  PageParam,
  LoaderDecl,
  LoaderRun,
  Node,
  Page,
  PageRun,
  PartView,
  RunNode,
  SeeAlso,
  SqlCell,
  TileSize,
} from "../../tauri-bridge/generated/bindings.js";

export function fakeViz(over: Partial<VizConfig> = {}): VizConfig {
  return {
    id: "acme/page#node",
    extension: "acme",
    slug: "page",
    title: "",
    viz: "table",
    params: [],
    columns: [],
    empty: null,
    chart: null,
    tree: null,
    timeline: null,
    steps: null,
    hunks: null,
    code: null,
    media: null,
    diff: null,
    editor: null,
    embed: null,
    terminal: null,
    markdown: null,
    form: null,
    custom: null,
    group: null,
    emphasis: null,
    depth: null,
    fills: false,
    actions: [],
    origin: { extension: "acme", source: null, running: "", deployment: null, bundled: false, runningUri: null },
    ...over,
  } as VizConfig;
}

export function fakeAction(over: Partial<PageAction> & { id: string }): PageAction {
  return {
    label: over.id,
    command: `acme.${over.id}`,
    input: {},
    row: false,
    group: null,
    form: false,
    fixed: [],
    choices: {},
    then: "stay",
    when: null,
    fact: null,
    does: null,
    primary: false,
    prominent: false,
    drag: false,
    bulk: false,
    key: null,
    edit: false,
    editField: null,
    oneRow: false,
    all: false,
    ...over,
  } as PageAction;
}

interface Common {
  title?: string | null;
  description?: string | null;
  hideTitle?: boolean;
  callout?: boolean;
  icon?: string | null;
  titleFrom?: string | null;
  collapsible?: boolean;
  link?: string | null;
  size?: TileSize | null;
}

const common = (id: string, o: Common = {}) => ({
  id,
  title: o.title ?? null,
  description: o.description ?? null,
  hideTitle: o.hideTitle ?? false,
  callout: o.callout ?? false,
  icon: o.icon ?? null,
  titleFrom: o.titleFrom ?? null,
  collapsible: o.collapsible ?? false,
  link: o.link ?? null,
  replaceable: null,
  size: o.size ?? null,
});

export const node = {
  viz: (id: string, from: string | null, page: Partial<VizConfig> = {}, o: Common = {}): Node =>
    ({ ...common(id, o), kind: "viz", from, config: fakeViz(page) }) as Node,
  stack: (id: string, children: Node[], o: Common = {}): Node => ({ ...common(id, o), kind: "stack", children }) as Node,
  grid: (id: string, children: Node[], o: Common = {}): Node => ({ ...common(id, o), kind: "grid", children }) as Node,
  switch: (id: string, param: string, children: Node[], o: Common = {}): Node =>
    ({ ...common(id, o), kind: "switch", param, children }) as Node,
  embed: (id: string, page: string, o: Common = {}): Node => ({ ...common(id, o), kind: "embed", page, params: {} }) as Node,
  slot: (id: string, name: string, o: Common = {}): Node => ({ ...common(id, o), kind: "slot", name }) as Node,
  markdown: (id: string, text: string, o: Common = {}): Node => ({ ...common(id, o), kind: "markdown", text }) as Node,
  component: (id: string, component: string, o: Common = {}): Node =>
    ({ ...common(id, o), kind: "component", component, props: null }) as Node,
};

export function fakePage(view: Node, over: Partial<Page> = {}): Page {
  return {
    id: "acme/page",
    extension: "acme",
    slug: "page",
    kind: "page",
    title: "Acme Page",
    description: "",
    label: null,
    icon: null,
    category: null,
    keywords: null,
    params: [],
    for: [],
    accepts: {},
    acceptsContent: null,
    default: false,
    pinned: false,
    comments: false,
    refresh: false,
    needs: [],
    actions: [],
    alert: null,
    tool: null,
    answers: [],
    badge: null,
    loaders: [],
    view,
    rail: null,
    origin: { extension: "acme", source: null, running: "", deployment: null, bundled: false, runningUri: null },
    ...over,
  } as Page;
}

/** A loader's declaration (what `labelRow` reads the order of). */
export function decl(name: string): LoaderDecl {
  return { name, loader: { kind: "rows", columns: [], rows: [] }, columns: [], reads: [], refresh: null, empty: null, readsLoaders: [], readsParams: [] };
}

export function loader(columns: string[], rows: SqlCell[][], error: string | null = null): LoaderRun {
  return {
    result: { columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] } as never, freshness: [] },
    error,
    key: JSON.stringify([columns, rows]),
  };
}

/** The tree the server would resolve for `n`: every child shown, a
 *  switch's first (or `pick`'s) only. */
export function runTreeOf(n: Node, pick: Record<string, string> = {}, embeds: Record<string, PageRun> = {}): RunNode {
  const kids = "children" in n && n.children ? n.children : [];
  let shown: Node[] = kids;
  if (n.kind === "switch") {
    const chosen = kids.find((c) => c.id === pick[n.id]) ?? kids[0];
    shown = chosen ? [chosen] : [];
  }
  return {
    id: n.id,
    children: shown.map((c) => runTreeOf(c, pick, embeds)),
    embed: n.kind === "embed" ? (embeds[n.id] ?? null) : null,
    error: null,
  } as RunNode;
}

export function fakeRun(
  page: Page,
  o: {
    params?: Record<string, SqlCell>;
    loaders?: Record<string, LoaderRun>;
    pick?: Record<string, string>;
    embeds?: Record<string, PageRun>;
    tree?: RunNode;
    seeAlso?: SeeAlso[];
  } = {},
): PageRun {
  return {
    page,
    params: o.params ?? {},
    loaders: o.loaders ?? {},
    tree: o.tree ?? runTreeOf(page.view, o.pick, o.embeds),
    rail: page.rail ? runTreeOf(page.rail, o.pick, o.embeds) : null,
    alert: null,
    badge: null,
    seeAlso: o.seeAlso ?? [],
  } as PageRun;
}

// --- The scenarios -------------------------------------------------------

/** A page that is one table over one loader (a page moved over 1:1). */
export function tablePage(over: { actions?: PageAction[]; params?: PageParam[]; rowActions?: PageAction[] } = {}): PageRun {
  const page = fakePage(
    node.viz("items", "items", {
      columns: [{ key: "id" }, { key: "title" }],
      actions: over.rowActions ?? [],
    }),
    { title: "Items", label: "Items: {{row.title}}", params: over.params ?? [], actions: over.actions ?? [], loaders: [decl("items")] },
  );
  return fakeRun(page, {
    loaders: {
      items: loader(
        ["id", "title"],
        [
          ["a", "Alpha"],
          ["b", "Beta"],
        ],
      ),
    },
  });
}

/** A stack: a markdown intro, a table, and another page embedded. */
export function stackPage(): PageRun {
  const inner = fakeRun(
    fakePage(node.viz("notes", "notes", { columns: [{ key: "note" }] }), { id: "acme/notes", title: "Notes" }),
    { loaders: { notes: loader(["note"], [["inner note"]]) } },
  );
  const page = fakePage(
    node.stack("stack", [
      node.markdown("intro", "Hello **there**", { title: "Intro" }),
      node.viz("items", "items", { columns: [{ key: "id" }] }, { title: "Items" }),
      node.embed("notes", "acme/notes", { title: "Notes" }),
      node.slot("more", "more"),
      node.component("chart", "acme:chart"),
    ]),
  );
  return fakeRun(page, { loaders: { items: loader(["id"], [["a"]]) }, embeds: { notes: inner } });
}

/** A switch over a `mode` param, showing `mode`'s child. */
export function switchPage(mode: "list" | "board" = "list"): PageRun {
  const page = fakePage(
    node.switch("modes", "mode", [
      node.viz("list", "items", { columns: [{ key: "id" }] }),
      node.viz("board", "items", { columns: [{ key: "id" }] }),
    ]),
    { params: [{ name: "mode", role: "filter", default: "list", type: "choice", options: [] }] },
  );
  return fakeRun(page, { params: { mode }, loaders: { items: loader(["id"], [["a"]]) }, pick: { modes: mode } });
}

/** A grid of two tiles, one whose loader failed. */
export function gridPage(): PageRun {
  const page = fakePage(
    node.grid("tiles", [
      node.viz("count", "count", { columns: [{ key: "n" }] }, { title: "Count", size: "small" }),
      node.viz("broken", "broken", { columns: [{ key: "n" }] }, { title: "Broken", size: "wide" }),
    ]),
  );
  return fakeRun(page, {
    loaders: { count: loader(["n"], [[3]]), broken: loader([], [], "no such table: v_nope") },
  });
}

/** A `PageRuns` store whose daemon answers with `run` (and `next` after
 *  new params), recording what it was asked. */
export function fakePageRuns(run: PageRun, next?: (params: Record<string, SqlCell>) => PageRun | Promise<PageRun>) {
  const calls = { open: [] as PageKey[], setParams: [] as Record<string, SqlCell>[], rerun: 0 };
  let change: (c: RunChange) => void = () => {};
  let seq = 0;
  const transport: PageRunsTransport = {
    open: async (key) => {
      calls.open.push(key);
      return { runId: "run-1", run };
    },
    close: async () => {},
    fetch: async () => ({ "run-1": { run, seq, panel: null } }),
    setParams: async (_id, params) => {
      calls.setParams.push(params);
      return next ? next(params) : run;
    },
    setVisible: async () => false,
    rerun: async () => {
      calls.rerun += 1;
      return run;
    },
    onChange: (h) => {
      change = h;
      return () => {};
    },
    onReconnect: () => () => {},
  };
  return { runs: createPageRuns(transport), calls, emit: (c: RunChange) => { seq = Math.max(seq, c.seq); change(c); } };
}

/** A `PageRuns` store over many pages: `answer` gives each key's run (a
 *  rejection: the open fails), each opened as `run-<n>`; what was opened
 *  is recorded. */
/** `view`: what a panel part's run gives its panel, as the daemon says it
 *  (`PartView`); none, a page's run (null). */
export function fakePageRunsBy(
  answer: (key: PageKey) => PageRun | Promise<PageRun>,
  view: (key: PageKey, run: PageRun) => PartView | null = () => null,
) {
  const opened: { runId: string; key: PageKey }[] = [];
  const byId = new Map<string, PageRun>();
  const views = new Map<string, PartView | null>();
  const transport: PageRunsTransport = {
    open: async (key) => {
      const run = await answer(key);
      const runId = `run-${opened.length + 1}`;
      opened.push({ runId, key });
      byId.set(runId, run);
      views.set(runId, view(key, run));
      return { runId, run, panel: views.get(runId)! };
    },
    close: async () => {},
    fetch: async (ids) =>
      Object.fromEntries(ids.flatMap((id) => (byId.has(id) ? [[id, { run: byId.get(id)!, seq: 0, panel: views.get(id) ?? null }]] : []))),
    setParams: async (id) => byId.get(id)!,
    rerun: async (id) => byId.get(id)!,
    setVisible: async () => false,
    onChange: () => () => {},
    onReconnect: () => () => {},
  };
  return { runs: createPageRuns(transport), opened };
}
