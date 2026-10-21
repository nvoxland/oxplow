import { useEffect, useMemo, useState } from "react";
import { Page } from "../tabs/Page.js";
import { TerminalPane } from "../components/TerminalPane.js";
import { TerminalTabStrip } from "../components/Terminal/TerminalTabStrip.js";
import {
  addTerminal,
  closeTerminal,
  commentTargetFor,
  defaultTerminalList,
  normalizeTerminalList,
  paneTargetFor,
  renameTerminal,
} from "../components/Terminal/terminalTabs.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { usePageMemory } from "../prefs/remembered.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import type { Stream } from "../api.js";

interface TerminalPageProps {
  stream: Stream | null;
  visible: boolean;
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
}

/**
 * The Terminal Page hosts one or more plain interactive shells rooted at
 * the stream's worktree dir. A left strip selects between them; each is a
 * `TerminalPane` with a `shell:<id>` pane target (the default terminal
 * keeps the bare `"shell"` target — see `terminalTabs.ts`). Panes are
 * kept warm via `display:none` so switching terminals preserves
 * scrollback; closing a terminal kills its shell (`terminateOnUnmount`).
 */
export function TerminalPage({ stream, visible, onOpenFile }: TerminalPageProps) {
  usePageTitle("Terminal");

  const streamId = stream?.id ?? null;
  // The terminals are this page's memory (every tab of it, each stream its
  // own); which one is open is this step of this tab's history.
  const [storedTabs, setStoredTabs] = usePageMemory<unknown>(`tabs.${streamId ?? ""}`, null);
  const [activeByStream, setActiveByStream] = useHistoryState<Record<string, string>>("terminalActive", {});
  // Ids the user has asked to close. A pane renders with
  // `terminateOnUnmount` while its id is here, so when the effect below
  // removes it from the list the unmount kills the PTY (vs the detach
  // that a stream-switch / page-close unmount performs).
  const [closingIds, setClosingIds] = useState<Set<string>>(() => new Set());

  const tabs = useMemo(
    () => (streamId && storedTabs ? normalizeTerminalList(storedTabs) : defaultTerminalList()),
    [streamId, storedTabs],
  );
  const rawActive = streamId ? activeByStream[streamId] : undefined;
  const activeId = tabs.some((t) => t.id === rawActive) ? (rawActive as string) : tabs[0].id;

  // Apply queued closes: run the reducer for each closing id, then clear
  // the queue. Runs after the render in which the closing panes saw
  // `terminateOnUnmount`, so their unmount kills the shell.
  useEffect(() => {
    if (!streamId || closingIds.size === 0) return;
    let nextTabs = tabs;
    let nextActive = activeId;
    for (const id of closingIds) {
      const r = closeTerminal(nextTabs, nextActive, id);
      nextTabs = r.list;
      nextActive = r.activeId;
    }
    setStoredTabs(nextTabs);
    setActiveByStream({ ...activeByStream, [streamId]: nextActive });
    setClosingIds(new Set());
  }, [closingIds, streamId, tabs, activeId, activeByStream, setStoredTabs, setActiveByStream]);

  function handleNew() {
    if (!streamId) return;
    const id = `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const r = addTerminal(tabs, id);
    setStoredTabs(r.list);
    setActiveByStream({ ...activeByStream, [streamId]: r.activeId });
  }

  function handleActivate(id: string) {
    if (!streamId) return;
    setActiveByStream({ ...activeByStream, [streamId]: id });
  }

  function handleRename(id: string, title: string) {
    if (!streamId) return;
    setStoredTabs(renameTerminal(tabs, id, title));
  }

  function handleClose(id: string) {
    setClosingIds((prev) => new Set(prev).add(id));
  }

  if (!stream || !streamId) {
    return (
      <Page testId="page-terminal" title="Terminal" kind="terminal">
        <div style={{ padding: 12, color: "var(--muted)" }}>No project open.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-terminal" title="Terminal" kind="terminal">
      <div style={{ display: "flex", flexDirection: "row", height: "100%", minHeight: 0 }}>
        <TerminalTabStrip
          tabs={tabs}
          activeId={activeId}
          onActivate={handleActivate}
          onNew={handleNew}
          onClose={handleClose}
          onRename={handleRename}
        />
        <div style={{ position: "relative", flex: 1, minWidth: 0, minHeight: 0 }}>
          {tabs.map((tab) => {
            const isActive = tab.id === activeId;
            return (
              <div
                key={`${stream.id}:${tab.id}`}
                style={{
                  position: "absolute",
                  inset: 0,
                  // A flex column, so the pane fills it (tsk1042).
                  display: isActive ? "flex" : "none",
                  flexDirection: "column",
                }}
              >
                <TerminalPane
                  paneTarget={paneTargetFor(tab.id)}
                  visible={visible && isActive}
                  worktreePath={stream.worktree_path}
                  onOpenFile={onOpenFile}
                  terminateOnUnmount={closingIds.has(tab.id)}
                  comments={{
                    streamId: stream.id,
                    threadId: null,
                    // URI-SWITCH: a terminal has no resource of its own yet.
                    target: commentTargetFor(stream.id, tab.id),
                  }}
                />
              </div>
            );
          })}
        </div>
      </div>
    </Page>
  );
}
