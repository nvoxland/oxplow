/// The Board page (`page:board`, P6.E1a): the work items of this thread,
/// the backlog, or everything, by canonical state (`WorkBoard`).
import { useState } from "react";

import { WorkBoard } from "../components/Board/WorkBoard.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { WorkItemScope } from "../workItems.js";

type ScopeChoice = "thread" | "backlog" | "all";

export function BoardPage({ threadId, onOpenPage }: { threadId: string | null; onOpenPage(ref: TabRef): void }) {
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
        <WorkBoard scope={scope} onOpenPage={onOpenPage} />
      </div>
    </Page>
  );
}
