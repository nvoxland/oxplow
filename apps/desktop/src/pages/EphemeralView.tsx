import { useState } from "react";
import type { Stream } from "../api.js";
import type { PageRuns } from "../pageRuns.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { PageHost } from "./PageHost.js";
import { pageView } from "./renderers.js";

/** A view's URI names an in-memory view, while history keeps its source
 *  so a restored tab or a daemon reconnect can reopen it after a restart. */
export function EphemeralView({ view, stream, threadId, onOpenPage, runs }: {
  view: string;
  stream: Stream | null;
  threadId: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  runs?: PageRuns;
}) {
  const nav = useOptionalPageNavigation();
  const [saved, keep] = useHistoryState<string | null>("view.specYaml", null);
  // The source returned from opening this run updates history without
  // opening a duplicate run. A restored component starts with that source.
  const [restored] = useState(saved);
  // `#node=` is where it lands, not a param of its run.
  const { node: focus, ...params } = pageView(nav?.viewParams ?? {});
  return <PageHost
    pageKey={{ view, ...(restored ? { specYaml: restored } : {}), params, streamId: stream?.id ?? null, threadId }}
    runs={runs}
    focus={typeof focus === "string" ? focus : null}
    stream={stream}
    onOpenPage={onOpenPage}
    onSpecYaml={keep}
    onChange={nav?.patchViewParams ? (patch) => nav.patchViewParams!(patch) : undefined}
  />;
}
