import { useEffect, useRef } from "react";
import { Page } from "../tabs/Page.js";
import { TerminalPane } from "../components/TerminalPane.js";
import { TerminalTabStrip } from "../components/Terminal/TerminalTabStrip.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { recordOpError } from "../opErrors.js";
import { closeTerminal, openTerminal, renameTerminal, useTerminals } from "../terminals.js";
import type { Stream } from "../api.js";

interface TerminalPageProps {
  stream: Stream | null;
  visible: boolean;
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
}

const fail = (label: string) => (e: unknown) =>
  recordOpError({ label, message: e instanceof Error ? e.message : String(e) });

/**
 * The Terminal Page hosts the stream's terminals: rows (`v_terminal`), each
 * a plain interactive shell rooted at the worktree and keyed by its URI. A
 * left strip selects between them; each is a `TerminalPane` on the row's
 * URI. Panes are kept warm via `display:none` so switching terminals
 * preserves scrollback. Closing one is a command that ends its shell; the
 * last may close, and the page offers New. A stream that has none when the
 * page first opens gets one.
 */
export function TerminalPage({ stream, visible, onOpenFile }: TerminalPageProps) {
  usePageTitle("Terminal");

  const streamId = stream?.id ?? null;
  const terminals = useTerminals(streamId);
  // Which one is shown is this step of this tab's history.
  const [activeByStream, setActiveByStream] = useHistoryState<Record<string, string>>("terminalActive", {});
  // A stream the page has seeded a first terminal for: never again, so
  // closing the last one leaves the page empty with New.
  const seeded = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (!streamId || terminals === null || terminals.length > 0 || seeded.current.has(streamId)) return;
    seeded.current.add(streamId);
    void openTerminal(streamId)
      .then((t) => setActiveByStream({ ...activeByStream, [streamId]: t.id }))
      .catch(fail("Open terminal"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamId, terminals]);

  const list = terminals ?? [];
  const rawActive = streamId ? activeByStream[streamId] : undefined;
  const active = list.find((t) => t.id === rawActive) ?? list[0] ?? null;

  function handleNew() {
    if (!streamId) return;
    void openTerminal(streamId)
      .then((t) => setActiveByStream({ ...activeByStream, [streamId]: t.id }))
      .catch(fail("Open terminal"));
  }

  function handleClose(id: string) {
    // The neighbour (previous, else next) takes over the shown place.
    const i = list.findIndex((t) => t.id === id);
    const next = list.filter((t) => t.id !== id);
    if (streamId && active?.id === id && next.length > 0) {
      setActiveByStream({ ...activeByStream, [streamId]: (next[Math.max(0, i - 1)] ?? next[0])!.id });
    }
    void closeTerminal(id).catch(fail("Close terminal"));
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
          tabs={list}
          activeId={active?.id ?? ""}
          onActivate={(id) => setActiveByStream({ ...activeByStream, [streamId]: id })}
          onNew={handleNew}
          onClose={handleClose}
          onRename={(id, title) => void renameTerminal(id, title).catch(fail("Rename terminal"))}
        />
        <div style={{ position: "relative", flex: 1, minWidth: 0, minHeight: 0 }}>
          {terminals !== null && list.length === 0 ? (
            <div style={{ padding: 12, color: "var(--muted)" }}>
              No terminal is open.{" "}
              <button type="button" className="ox-link" data-testid="terminal-new" onClick={handleNew}>
                New Terminal
              </button>
            </div>
          ) : null}
          {list.map((tab) => {
            const isActive = tab.id === active?.id;
            return (
              <div
                key={tab.uri}
                style={{
                  position: "absolute",
                  inset: 0,
                  // A flex column, so the pane fills it (tsk1042).
                  display: isActive ? "flex" : "none",
                  flexDirection: "column",
                }}
              >
                <TerminalPane
                  uri={tab.uri}
                  visible={visible && isActive}
                  worktreePath={stream.worktree_path}
                  onOpenFile={onOpenFile}
                  comments={{ streamId: stream.id, threadId: null, target: tab.uri }}
                />
              </div>
            );
          })}
        </div>
      </div>
    </Page>
  );
}
