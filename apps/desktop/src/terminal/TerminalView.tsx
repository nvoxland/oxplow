/// One terminal on screen: its surface, its session, and what the session
/// says when it isn't running — ended or failed to start
/// (Start Again / Retry), or unable to resume a saved conversation (Retry,
/// Start Fresh). On a touch screen, the keys an on-screen keyboard lacks
/// (`TerminalTouchBar`). The agent's parts are `agent`'s; a shell has none.
import { useRef, useState, type CSSProperties } from "react";
import type { Terminal } from "@xterm/xterm";

import { useCoarsePointer } from "../coarsePointer.js";
import { canTakeSize, exitBanner, MIN_TERMINAL } from "../components/terminalSession.js";
import { TerminalSurface } from "./TerminalSurface.js";
import { TerminalTouchBar } from "./TerminalTouchBar.js";
import { ctrlOf } from "./touchKeys.js";
import { useShown } from "./useShown.js";
import { useTerminalSession } from "./useTerminalSession.js";

export interface AgentTerminal {
  /** What it is bound to: a change opens it again. */
  conversation: string | null;
  /** Begin a new conversation, the saved one dropped (confirmed here). */
  onStartFresh(): Promise<void>;
  /** The person pressed Escape: the host records the interrupt. */
  onInterrupt(): void;
}

export interface TerminalViewProps {
  /** The PTY it shows: an `oxplow/terminal` or `oxplow/agent_session` URI. */
  uri: string;
  agent?: AgentTerminal;
  worktreePath?: string;
  onOpenFile?(absPath: string, line?: number, column?: number): void;
  isLinkablePath?(path: string): boolean;
}

export function TerminalView({ uri, agent, worktreePath, onOpenFile, isLinkablePath }: TerminalViewProps) {
  const boxRef = useRef<HTMLDivElement>(null);
  const shown = useShown(boxRef);
  const session = useTerminalSession(uri, {
    startOnActivate: !!agent,
    shown,
    conversation: agent?.conversation ?? null,
  });
  const detach = useRef<(() => void) | null>(null);
  const term = useRef<Terminal | null>(null);
  const [confirmFresh, setConfirmFresh] = useState(false);
  const touch = useCoarsePointer();
  // The touch bar's Ctrl: it applies to the next key typed, then turns off.
  const [ctrl, setCtrl] = useState(false);
  const ctrlOn = useRef(false);
  ctrlOn.current = ctrl;
  const banner = session.ended ? exitBanner(session.ended) : null;
  // The cells this pane has room for. The daemon says whether another
  // client owns the terminal's size (it typed into it last); then, when the
  // grid differs, Fit to this device takes it.
  const [pane, setPane] = useState<{ cols: number; rows: number } | null>(null);
  const grid = session.grid;
  const elsewhere = session.ownedElsewhere && !!grid && !!pane && (grid.cols !== pane.cols || grid.rows !== pane.rows);
  return (
    <div ref={boxRef} data-testid="terminal-view" data-uri={uri} style={boxStyle}>
      <TerminalSurface
        shown={shown}
        uri={uri}
        grid={grid}
        onOpened={(opened, room) => {
          term.current = opened;
          setPane(room);
          detach.current?.();
          detach.current = session.attach(opened, room);
        }}
        onData={(data, typed) => {
          if (ctrlOn.current && typed) {
            setCtrl(false);
            ctrlOn.current = false;
            session.input(ctrlOf(data) ?? data, true);
            return;
          }
          session.input(data, typed);
        }}
        onBinary={session.binary}
        onResize={(cols, rows) => {
          setPane({ cols, rows });
          session.resize(cols, rows);
        }}
        onInterrupt={agent?.onInterrupt}
        worktreePath={worktreePath}
        cwd={session.cwd}
        onOpenFile={onOpenFile}
        isLinkablePath={isLinkablePath}
      />
      {elsewhere ? (
        // A chip in the corner: in the flow it would change the pane's size,
        // and over the bottom it would cover the keys and the input box.
        <div data-testid="terminal-sized-elsewhere" style={chipStyle}>
          <span>
            Sized for another device ({grid!.cols}×{grid!.rows}).
          </span>
          <button
            type="button"
            data-testid="terminal-fit"
            disabled={!canTakeSize(pane!)}
            title={
              canTakeSize(pane!)
                ? undefined
                : `This pane is too small to take the size: it needs ${MIN_TERMINAL.cols}×${MIN_TERMINAL.rows} cells, it has ${pane!.cols}×${pane!.rows}.`
            }
            onClick={() => session.fit(pane!.cols, pane!.rows)}
          >
            Fit to this device
          </button>
          {canTakeSize(pane!) ? null : (
            <span data-testid="terminal-fit-refused">
              Too small to fit ({MIN_TERMINAL.cols}×{MIN_TERMINAL.rows} at least).
            </span>
          )}
        </div>
      ) : null}
      {touch ? (
        <TerminalTouchBar
          send={(bytes) => session.input(bytes, true)}
          applicationCursor={() => term.current?.modes.applicationCursorKeysMode ?? false}
          ctrl={ctrl}
          onCtrl={setCtrl}
          onEscape={agent?.onInterrupt}
          onInsert={(text) => {
            // A paste: bracketed when the program asked for it, and no Enter.
            term.current?.paste(text);
            term.current?.focus();
          }}
        />
      ) : null}
      {banner ? (
        <div data-testid="terminal-ended" style={bannerStyle}>
          <span>{banner.text}</span>
          {banner.actions.map((action) =>
            action === "start-fresh" ? (
              agent ? (
                <button
                  key={action}
                  type="button"
                  data-testid="terminal-start-fresh"
                  title="Begin a new conversation. The saved one is no longer resumed."
                  onClick={() => {
                    if (!confirmFresh) {
                      setConfirmFresh(true);
                      return;
                    }
                    setConfirmFresh(false);
                    void agent.onStartFresh().then(() => session.reopen());
                  }}
                >
                  {confirmFresh ? "Confirm: drop the saved conversation" : "Start Fresh"}
                </button>
              ) : null
            ) : (
              <button
                key={action}
                type="button"
                data-testid={action === "retry" ? "terminal-retry" : "terminal-start-again"}
                onClick={() => session.reopen()}
              >
                {action === "retry" ? "Retry" : "Start Again"}
              </button>
            ),
          )}
        </div>
      ) : null}
    </div>
  );
}

const boxStyle: CSSProperties = { position: "relative", flex: 1, minHeight: 0, display: "flex", flexDirection: "column" };
const chipStyle: CSSProperties = {
  position: "absolute",
  top: 4,
  right: 12,
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "3px 8px",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  color: "var(--text-primary)",
  fontSize: 12,
  zIndex: 4,
};
const bannerStyle: CSSProperties = {
  position: "absolute",
  left: 0,
  right: 0,
  bottom: 0,
  display: "flex",
  alignItems: "center",
  gap: 10,
  padding: "8px 12px",
  background: "var(--surface-card)",
  borderTop: "1px solid var(--border-subtle)",
  color: "var(--text-primary)",
  fontSize: 13,
  zIndex: 4,
};
