import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { isMap, isSeq, parseDocument } from "yaml";

import { useSelectedThreadId } from "../../contextBasket.js";
import { useVizStream } from "../../viz/VizStream.js";
import { Loading } from "../../kit/Loading.js";
import { usePageRunsStore, usePageRunWith } from "../../pageRuns.js";
import { NodeRegistryContext, PageTree } from "../../pages/PageTree.js";
import { PageViewContext } from "../../pages/PageViewContext.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import type { SqlCell } from "../../tauri-bridge/generated/bindings.js";

/** A markdown block opens a read-only inline-spec run, never a deployed
 * page's chrome. The daemon applies spec_problem before running loaders. */
export const INLINE_PAGE_MAX_BYTES = 64 * 1024;
export const INLINE_PAGE_MAX_DEPTH = 4;
const InlineDepth = createContext(0);

export interface InlineViewProps {
  specYaml: string;
  streamId?: string | null;
  threadId?: string | null;
}

export function InlineView(props: InlineViewProps) {
  const depth = useContext(InlineDepth);
  // Bound recursive markdown (a page's markdown node can itself contain
  // a page fence), as well as the source sent to the daemon.
  const problem = depth >= INLINE_PAGE_MAX_DEPTH
    ? "Inline page nesting is too deep."
    : new TextEncoder().encode(props.specYaml).length > INLINE_PAGE_MAX_BYTES
      ? "Inline page source exceeds 64 KiB."
      : null;
  if (problem) return <InlineError message={problem} source={props.specYaml} />;
  return <InlineDepth.Provider value={depth + 1}><InlineRun {...props} /></InlineDepth.Provider>;
}

function InlineRun({ specYaml, streamId, threadId }: InlineViewProps) {
  const stream = useVizStream();
  const thread = useSelectedThreadId();
  const scope = useContext(PageViewContext);
  const resolvedStream = streamId === undefined ? scope ? scope.streamId : stream?.id ?? null : streamId;
  const resolvedThread = threadId === undefined ? scope ? scope.threadId : thread : threadId;
  const nav = useOptionalPageNavigation();
  // The containing URI keeps a block's picks and filters too. Pass only
  // names this spec declares: its host's other view keys are not params
  // of the inline page and the daemon correctly refuses unknown names.
  const params = useMemo(() => {
    const out: Record<string, SqlCell> = {};
    const declared = parseDocument(specYaml).get("params");
    if (isSeq(declared)) for (const item of declared.items) {
      const name = isMap(item) ? item.get("name") : null;
      const value = typeof name === "string" ? nav?.viewParams?.[name] : undefined;
      if (typeof name === "string" && value !== undefined) out[name] = value;
    }
    return out;
  }, [specYaml, nav?.viewParams]);
  const runs = usePageRunsStore();
  const state = usePageRunWith(runs, {
    specYaml,
    params,
    streamId: resolvedStream,
    threadId: resolvedThread,
  });
  const [reading, setReading] = useState(false);
  useEffect(() => {
    setReading(false);
    if (state.run || state.error) return;
    const timer = setTimeout(() => setReading(true), 150);
    return () => clearTimeout(timer);
  }, [specYaml, state.run, state.error]);
  if (state.error) return <InlineError message={state.error} source={specYaml} />;
  if (!state.run) return reading ? <Loading what="view" /> : null;
  return (
    <div data-testid="inline-page" aria-busy={state.stale}>
      {/* Node ids belong to this run, never the containing wiki's. */}
      <NodeRegistryContext.Provider value={null}>
        <PageTree
          run={state.run}
          runId={state.runId}
          streamId={resolvedStream}
          onOpenPage={nav?.navigate}
          onSetParam={(name, value) => {
            const pick = state.run?.page.params.find((param) => param.name === name)?.role === "selection";
            nav?.patchViewParams?.({ [name]: value }, { push: pick });
            void state.setParams({ [name]: value });
          }}
        />
      </NodeRegistryContext.Provider>
    </div>
  );
}

function InlineError({ message, source }: { message: string; source: string }) {
  return (
    <div data-testid="inline-page-error" className="ox-error" role="alert">
      <pre>{source.length > INLINE_PAGE_MAX_BYTES ? `${source.slice(0, INLINE_PAGE_MAX_BYTES)}…` : source}</pre>
      <p>{message}</p>
    </div>
  );
}
