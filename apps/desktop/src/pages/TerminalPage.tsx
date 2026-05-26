import { useEffect, useState } from "react";
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
  type TerminalTab,
} from "../components/Terminal/terminalTabs.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { logUi } from "../logger.js";
import type { Stream } from "../api.js";

interface TerminalPageProps {
  stream: Stream | null;
  visible: boolean;
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
}

// Per-stream layout, mirroring App.tsx's `oxplow.layout.v1.*` blobs:
// one `Record<streamId, …>` per key with defensive parse + try/catch.
const TABS_KEY = "oxplow.layout.v1.terminalTabs";
const ACTIVE_KEY = "oxplow.layout.v1.terminalActive";

function readTabsByStream(): Record<string, TerminalTab[]> {
  try {
    const raw = localStorage.getItem(TABS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const out: Record<string, TerminalTab[]> = {};
    for (const [streamId, list] of Object.entries(parsed)) {
      out[streamId] = normalizeTerminalList(list);
    }
    return out;
  } catch (e) {
    logUi("warn", "failed to read persisted terminal tabs", { error: String(e) });
    return {};
  }
}

function readActiveByStream(): Record<string, string> {
  try {
    const raw = localStorage.getItem(ACTIVE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const out: Record<string, string> = {};
    for (const [streamId, id] of Object.entries(parsed)) {
      if (typeof id === "string") out[streamId] = id;
    }
    return out;
  } catch {
    return {};
  }
}

function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // best-effort persistence
  }
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

  const [tabsByStream, setTabsByStream] = useState<Record<string, TerminalTab[]>>(readTabsByStream);
  const [activeByStream, setActiveByStream] = useState<Record<string, string>>(readActiveByStream);
  // Ids the user has asked to close. A pane renders with
  // `terminateOnUnmount` while its id is here, so when the effect below
  // removes it from the list the unmount kills the PTY (vs the detach
  // that a stream-switch / page-close unmount performs).
  const [closingIds, setClosingIds] = useState<Set<string>>(() => new Set());

  const streamId = stream?.id ?? null;
  const tabs = streamId ? (tabsByStream[streamId] ?? defaultTerminalList()) : defaultTerminalList();
  const rawActive = streamId ? activeByStream[streamId] : undefined;
  const activeId = tabs.some((t) => t.id === rawActive) ? (rawActive as string) : tabs[0].id;

  useEffect(() => {
    writeJson(TABS_KEY, tabsByStream);
  }, [tabsByStream]);
  useEffect(() => {
    writeJson(ACTIVE_KEY, activeByStream);
  }, [activeByStream]);

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
    setTabsByStream((m) => ({ ...m, [streamId]: nextTabs }));
    setActiveByStream((m) => ({ ...m, [streamId]: nextActive }));
    setClosingIds(new Set());
  }, [closingIds, streamId, tabs, activeId]);

  function handleNew() {
    if (!streamId) return;
    const id = `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const r = addTerminal(tabs, id);
    setTabsByStream((m) => ({ ...m, [streamId]: r.list }));
    setActiveByStream((m) => ({ ...m, [streamId]: r.activeId }));
  }

  function handleActivate(id: string) {
    if (!streamId) return;
    setActiveByStream((m) => ({ ...m, [streamId]: id }));
  }

  function handleRename(id: string, title: string) {
    if (!streamId) return;
    setTabsByStream((m) => ({ ...m, [streamId]: renameTerminal(m[streamId] ?? tabs, id, title) }));
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
                  display: isActive ? "block" : "none",
                }}
              >
                <TerminalPane
                  paneTarget={paneTargetFor(tab.id)}
                  visible={visible && isActive}
                  transportMode="direct"
                  worktreePath={stream.worktree_path}
                  onOpenFile={onOpenFile}
                  terminateOnUnmount={closingIds.has(tab.id)}
                  comments={{
                    streamId: stream.id,
                    threadId: null,
                    targetKind: "terminal",
                    targetId: commentTargetFor(stream.id, tab.id),
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
