/// A shown view in a tab (`.context/pages-and-tabs.md` "Views"): the page
/// `temp/<id>`, a page file on `oxplow:temp`. The window doesn't have it in
/// its extensions, so it asks the daemon for the page, then draws it like
/// any page file (`PageFileView`). A file the sweep removed (or that was
/// never written) is said in its place.
import { useEffect, useState } from "react";

import { unwrapResult } from "../api.js";
import type { Stream } from "../api.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { IpcCallError } from "../ipc-error.js";
import type { PageRuns } from "../pageRuns.js";
import { Page as Chrome } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { commands, type Page, type SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageFileView } from "./PageFileView.js";
import { TEMP_EXTENSION } from "./viewActions.js";

/** Asks the daemon for a page by id (`get_page`). */
export type GetPage = (id: string) => Promise<Page>;
export const daemonGetPage: GetPage = async (id) => unwrapResult(await commands.getPage(id));

type Loaded = { page: Page } | { gone: string | null };

export function TempPage({ id, resource, stream, threadId, onOpenPage, runs, getPage = daemonGetPage }: {
  id: string;
  resource: Record<string, SqlCell>;
  stream: Stream | null;
  threadId: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  runs?: PageRuns;
  getPage?: GetPage;
}) {
  const [loaded, setLoaded] = useState<{ id: string; result: Loaded } | null>(null);
  useEffect(() => {
    let live = true;
    getPage(`${TEMP_EXTENSION}/${id}`).then(
      (page) => live && setLoaded({ id, result: { page } }),
      // Not found: the file is gone. Any other answer (it no longer
      // parses) says what it is.
      (e: unknown) =>
        live &&
        setLoaded({
          id,
          result: { gone: e instanceof IpcCallError && e.code === "NOT_FOUND" ? null : e instanceof Error ? e.message : String(e) },
        }),
    );
    return () => { live = false; };
  }, [id, getPage]);
  if (!loaded || loaded.id !== id) return null;
  if ("page" in loaded.result) {
    return <PageFileView page={loaded.result.page} resource={resource} stream={stream} threadId={threadId} onOpenPage={onOpenPage} runs={runs} />;
  }
  return (
    <Chrome testId="view-gone" title="View">
      <div style={{ padding: 16 }}>
        <EmptyState title="This view is gone" text={loaded.result.gone ?? "oxplow keeps a view for a week after you last opened it. Ask the agent to show it again."} />
      </div>
    </Chrome>
  );
}
