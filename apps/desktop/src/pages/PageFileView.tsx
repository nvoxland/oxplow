/// A page file in a tab (`.context/pages-and-tabs.md` "Page files", "The
/// desktop host"): the tab's entry is what it shows. Its run's params are
/// the entry's URI — the `?` params (a renderer's: `uri` and the
/// resource's own) and the filters and picks of its `#` — so a change of
/// view is a new key and Back and Forward show each entry's run. A change
/// the page makes goes back through the tab: a filter replaces the entry,
/// a pick or a section pushes one, another resource navigates.
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { withResourceParams } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Page, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { PageRuns } from "../pageRuns.js";
import type { Stream } from "../api.js";
import { PageHost } from "./PageHost.js";
import { pageRunParams, pageViewChange } from "./pageFiles.js";
import { pageView, shellView, withRenderedParams } from "./renderers.js";

export interface PageFileViewProps {
  page: Page;
  /** What its URI's `?` names: a route's params, or a renderer's `uri`
   *  and its resource's params. */
  resource: Record<string, SqlCell>;
  /** The resource URI it renders (a renderer's). */
  renders?: string | null;
  /** The stream it shows, whole (the editor reads its worktree). */
  stream: Stream | null;
  threadId: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  /** The window's runs (default); a test's fake. */
  runs?: PageRuns;
}

export function PageFileView({ page, resource, renders = null, stream, threadId, onOpenPage, runs }: PageFileViewProps) {
  const streamId = stream?.id ?? null;
  const nav = useOptionalPageNavigation();
  const view = pageView(nav?.viewParams ?? NO_VIEW);
  const params = pageRunParams(page, resource, view);
  const focus = typeof view.node === "string" && view.node !== "" ? view.node : null;

  const onChange = nav?.setViewParams
    ? (patch: Record<string, SqlCell>) => {
        const change = pageViewChange(page, params, patch);
        // Whether it still shows the same thing: its `?` as the URI holds
        // it, defaults left out on both sides.
        const now = pageViewChange(page, resource, {}).resource;
        if (sameValues(change.resource, now) || !nav.ref) {
          nav.setViewParams!(change.view, { push: change.push });
        } else if (renders) {
          nav.navigate({ id: withRenderedParams(renders, change.resource, { ...shellView(nav.viewParams ?? NO_VIEW), ...change.view }) });
        } else {
          nav.navigate(withResourceParams(nav.ref, change.resource, change.view));
        }
      }
    : undefined;

  return (
    <PageHost
      pageKey={{ page: page.id, params, streamId, threadId }}
      runs={runs}
      focus={focus}
      onOpenPage={onOpenPage}
      onChange={onChange}
      renders={renders}
      stream={stream}
    />
  );
}

const NO_VIEW: Record<string, SqlCell> = {};

function sameValues(a: Record<string, SqlCell>, b: Record<string, SqlCell>): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) {
    const x = a[k] ?? null;
    const y = b[k] ?? null;
    if ((x === null) !== (y === null) || String(x) !== String(y)) return false;
  }
  return true;
}
