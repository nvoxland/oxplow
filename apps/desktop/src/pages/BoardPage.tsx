/// The Board page (`page:board`, P6.E1a): the work items of this thread,
/// the backlog, or everything, by canonical state (`WorkBoard`). The
/// columns of cards are replaceable (`work_item.board`, P9.A1): the active
/// work-items provider's extension may put its own lens there, given the
/// scope; the page and its scope picker stay oxplow's.
import { useState } from "react";

import { WorkBoard } from "../components/Board/WorkBoard.js";
import { Replaceable } from "../components/Replaceable.js";
import { threadRowId } from "../modelIds.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { WorkItemScope } from "../workItems.js";

type ScopeChoice = "thread" | "backlog" | "all";

export function BoardPage({
  threadId,
  streamId,
  onOpenPage,
}: {
  threadId: string | null;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [choice, setChoice] = useState<ScopeChoice>(threadId ? "thread" : "all");
  const scope: WorkItemScope = choice === "thread" && threadId ? { thread: threadId } : choice === "backlog" ? "backlog" : "all";
  return (
    <Page testId="page-board" title="Board" kind="board">
      <div style={{ display: "flex", flexDirection: "column", minHeight: 0, height: "100%" }}>
        <div style={{ padding: "8px 12px 0", fontSize: "var(--text-sm)" }}>
          <label>
            Show{" "}
            <select data-testid="board-scope" value={choice} onChange={(e) => setChoice(e.target.value as ScopeChoice)}>
              {threadId ? <option value="thread">This thread</option> : null}
              <option value="backlog">Backlog</option>
              <option value="all">Everything</option>
            </select>
          </label>
        </div>
        <Replaceable
          target="work_item.board"
          props={{
            scope: scope === "backlog" || scope === "all" ? scope : "thread",
            thread_id: typeof scope === "object" ? threadRowId(scope.thread) : null,
          }}
          streamId={streamId}
          onOpenPage={onOpenPage}
          fallback={<WorkBoard scope={scope} onOpenPage={onOpenPage} />}
        />
      </div>
    </Page>
  );
}
