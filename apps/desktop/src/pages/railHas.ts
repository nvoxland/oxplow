/// Whether a page's rail has anything to draw, decided from its run before
/// anything is drawn (`.context/usability.md` "A page's controls are in its
/// rail"): the "Details" card exists only when something is in it.
import { CONTENTS_FROM } from "../viz/Contents.js";
import { nodeById } from "../viz/nodeRun.js";
import type { Node, PageRun, RunNode } from "../tauri-bridge/generated/bindings.js";

/** The titled sections a page draws that Contents would list: a stack's
 *  or slot's children with a title (a hidden heading counts: it stays in
 *  the page for Contents; an empty section stays too). Mirrors
 *  `PageTree`'s drawing of `placement="section"`, and counts a
 *  `sections` or `settings` viz's subsections, every row. */
function sectionCount(run: PageRun): number {
  const walk = (run: PageRun, at: RunNode): number => {
    const def = nodeById(run.page, at.id);
    if (!def) return 0;
    let n = 0;
    // Row-based section renderers draw one shared Section per row.
    if (def.kind === "viz" && (def.config.viz === "sections" || def.config.viz === "settings")) {
      const result = def.from !== null ? run.loaders[def.from]?.result : undefined;
      n += result?.rows.length ?? 0;
      const group = def.config.sections?.group ? result?.columns.indexOf(def.config.sections.group) : undefined;
      if (result && group !== undefined && group >= 0) {
        n += new Set(result.rows.map((r) => r[group]).filter((v) => v !== null && v !== "")).size;
      }
    }
    if (def.kind === "stack" || def.kind === "slot") {
      n += at.children.filter((c) => titleOf(run, c)).length;
    }
    for (const c of at.children) n += walk(run, c);
    if (def.kind === "embed" && at.embed) n += walk(at.embed, at.embed.tree);
    return n;
  };
  return walk(run, run.tree);
}

function titleOf(run: PageRun, at: RunNode): string | null {
  const def = nodeById(run.page, at.id);
  return def?.title || (def?.kind === "embed" || !def ? (at.embed?.page.title ?? null) : null);
}

/** The vizzes that take the page's height when they are all of it: a
 *  history list should not sit five rows tall in an empty page. A custom
 *  component isn't one: it fills only when it declares `size: fill` (the
 *  run then marks its viz `fills`), else it is as tall as what it draws. */
const FILLS_ALONE: ReadonlySet<string> = new Set(["table", "list", "tree"]);

/** The one node the run shows as its page's body: its view, or the only
 *  child the run shows of a view that is a stack (a node's `when` keeps the
 *  others out of the run). */
function wholeNode(run: PageRun): string | null {
  const tree = run.tree;
  if (run.page.view.kind !== "stack") return tree.id;
  return tree.children.length === 1 ? tree.children[0]!.id : null;
}

/** Whether node `id` is the whole of its page's body. */
function isWholePage(run: PageRun, id: string): boolean {
  return run.tree.id === id || wholeNode(run) === id;
}

/** Whether viz node `id` takes its page's height: its viz always does (a
 *  file, a terminal, a component that declares `size: fill`), or it is a
 *  table, list or tree that is the whole page. `PageTree` fills the node and `PageHost` the body
 *  around it by this one rule. */
export function vizFills(run: PageRun, id: string): boolean {
  const def = nodeById(run.page, id);
  return def?.kind === "viz" && (def.config.fills || (FILLS_ALONE.has(def.config.viz) && isWholePage(run, id)));
}

/** Whether the page's body is one viz that fills only because it is all of
 *  the page (`vizFills` and not `viewFills`): the body takes the height, the
 *  page keeps its width and rail. */
export function aloneFills(run: PageRun): boolean {
  const only = wholeNode(run);
  return only !== null && vizFills(run, only);
}

/** Whether the page's body may have markdown headings Contents lists: a
 *  markdown viz (its text is data the run can't read here) or static
 *  markdown with an `##` or `###` heading. */
function mayHaveHeadings(run: PageRun): boolean {
  const walk = (n: Node): boolean =>
    (n.kind === "viz" && n.config.viz === "markdown") ||
    (n.kind === "markdown" && /^#{2,3}\s/m.test(n.text)) ||
    ((n.kind === "stack" || n.kind === "grid" || n.kind === "switch") && n.children.some(walk));
  return walk(run.page.view);
}

/** Whether one of the run's viz nodes fills (a file, a terminal, a
 *  diff): its page says. Such a page has no automatic Contents. */
export function viewFills(run: PageRun): boolean {
  const walk = (n: Node): boolean =>
    n.kind === "viz" ? n.config.fills : n.kind === "stack" || n.kind === "grid" || n.kind === "switch" ? n.children.some(walk) : false;
  return walk(run.page.view);
}

/** Whether Contents would list anything: not on a page that fills, else
 *  on a page with `CONTENTS_FROM` or more sections or markdown headings. */
export function contentsHas(run: PageRun, fills: boolean): boolean {
  return !fills && (sectionCount(run) >= CONTENTS_FROM || mayHaveHeadings(run));
}

/** Whether the rail has anything: a switch's sections, a `filter` param
 *  (a `view` param rides the `#` and is never drawn), a `selection`
 *  param (not on a page that fills: its viz draws the picks, as a
 *  terminal's tabs do), a `rail:` node, a see-also link or another view of the tab's item (`views`; not on a page that fills: its ☰ lists them), the save-a-view form (`saveOpen`), or Contents. */
export function railHas(run: PageRun, opts: { fills: boolean; saveOpen: boolean; views?: boolean }): boolean {
  const { params, view } = run.page;
  return (
    view.kind === "switch" ||
    params.some((p) => p.role === "filter" || (p.role === "selection" && !opts.fills)) ||
    run.rail !== null ||
    ((run.seeAlso.length > 0 || !!opts.views) && !opts.fills) ||
    opts.saveOpen ||
    contentsHas(run, opts.fills)
  );
}
